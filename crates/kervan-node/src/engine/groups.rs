//! Groups (§5.7): the admin's state, sender chains and group bundles.

use kervan_proto::bundle::{Bundle, Kind, Message, DEFAULT_HOP_LIMIT, DEFAULT_TTL_SECS};
use kervan_proto::frame::Frame;
use kervan_proto::group::{
    Distribution, GroupId, GroupState, ReceiverChain, SenderChain, MAX_GROUP_NAME_LEN, MAX_MEMBERS,
};
use kervan_proto::identity::ContactCard;
use kervan_proto::pair::PairKeys;
use kervan_proto::ratchet::Session;
use rand_core::OsRng;

use super::{forwarded, Contact, Engine, Handled, Outcome};
use crate::store::{StoredGroup, StoredGroupMessage};
use crate::{Error, Event, GroupMessageView, GroupView, MemberView, Result};

impl Engine {
    pub fn groups(&self) -> Result<Vec<GroupView>> {
        self.store
            .groups()?
            .iter()
            .map(|group| self.group_view(group))
            .collect()
    }

    pub fn group_messages(&self, group_id: i64) -> Result<Vec<GroupMessageView>> {
        let group = self.stored_group(group_id)?;
        Ok(self
            .store
            .group_messages(group.id)?
            .into_iter()
            .map(|message| self.group_message_view(&group, message))
            .collect())
    }

    /// Creates a group with us as its admin. The members get its state and
    /// our chain with the next send.
    pub fn create_group(&mut self, name: &str, contact_ids: &[i64], now: u32) -> Result<GroupView> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("choose a name for the group"));
        }
        if name.len() > MAX_GROUP_NAME_LEN {
            return Err(Error::Invalid("that name is too long for a group"));
        }
        let mut members = vec![self.identity.card()];
        for &contact_id in contact_ids {
            let card = self.joinable_card(contact_id)?;
            if !members
                .iter()
                .any(|member| member.dh_key() == card.dh_key())
            {
                members.push(card);
            }
        }
        check_size(&members)?;
        let state = GroupState {
            id: GroupId::generate(&mut OsRng),
            version: 1,
            name: name.to_owned(),
            members,
        };
        let id = self.publish_state(&state, &[], now)?;
        self.group_view(&self.stored_group(id)?)
    }

    pub fn add_group_member(
        &mut self,
        group_id: i64,
        contact_id: i64,
        now: u32,
    ) -> Result<GroupView> {
        let group = self.admin_group(group_id)?;
        let card = self.joinable_card(contact_id)?;
        if group.has_member(card.dh_key().as_bytes()) {
            return Err(Error::Invalid("already in the group"));
        }
        let mut members = group.members.clone();
        members.push(card);
        check_size(&members)?;
        self.change_members(&group, members, now)
    }

    /// Removes a member, named by fingerprint.
    pub fn remove_group_member(
        &mut self,
        group_id: i64,
        fingerprint: &str,
        now: u32,
    ) -> Result<GroupView> {
        let group = self.admin_group(group_id)?;
        let me = self.identity.dh_public();
        let members: Vec<ContactCard> = group
            .members
            .iter()
            .filter(|card| card.dh_key() == &me || card.fingerprint().to_string() != fingerprint)
            .cloned()
            .collect();
        if members.len() == group.members.len() {
            return Err(Error::Invalid("not a member of the group"));
        }
        self.change_members(&group, members, now)
    }

    /// Leaves a group: tells its admin and forgets its chains.
    pub fn leave_group(&mut self, group_id: i64, now: u32) -> Result<GroupView> {
        let group = self.stored_group(group_id)?;
        if !group.member {
            return Err(Error::Invalid("you are not in this group"));
        }
        if group.admin == *self.identity.dh_public().as_bytes() {
            return Err(Error::Invalid(
                "the admin cannot leave: remove the other members instead",
            ));
        }
        let admin = self.peer_index(&group.admin)?;
        self.send_pair(admin, &Message::leave(group.group_id, now), now)?;
        let state = GroupState {
            id: group.group_id,
            version: group.version,
            name: group.name.clone(),
            members: group.members.clone(),
        };
        self.store.save_group(&state, &group.admin, false, now)?;
        self.store.delete_group_chains(&group.group_id)?;
        self.forget_tags();
        self.group_view(&self.stored_group(group_id)?)
    }

    pub fn send_group_text(
        &mut self,
        group_id: i64,
        text: &str,
        now: u32,
    ) -> Result<(GroupMessageView, Frame)> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::Invalid("message is empty"));
        }
        let group = self.stored_group(group_id)?;
        if !group.member {
            return Err(Error::Invalid("you are no longer in this group"));
        }
        let (chain_id, mut chain) = self
            .store
            .own_chain(&group.group_id)?
            .ok_or(Error::Invalid("no sender chain for this group"))?;
        let bundle = chain.seal(
            &Message::text(text, now),
            now + DEFAULT_TTL_SECS,
            DEFAULT_HOP_LIMIT,
            &mut OsRng,
        )?;
        self.store.save_chain(chain_id, &chain.to_bytes())?;
        self.store.insert_bundle(&bundle, true, now)?;
        let id = self
            .store
            .insert_group_message(group.id, None, bundle.id, text, now, None)?
            .ok_or(Error::Invalid("message already exists"))?;
        let message = GroupMessageView {
            id,
            group_id: group.id,
            outgoing: true,
            sender: None,
            body: text.to_owned(),
            created_at: now,
            read: true,
        };
        let frame = forwarded(&bundle).expect("a new bundle has hops left");
        Ok((message, frame))
    }

    pub fn mark_group_read(&mut self, group_id: i64, now: u32) -> Result<()> {
        let group = self.stored_group(group_id)?;
        self.store.mark_group_read(group.id, now)
    }

    /// A group state from a contact (§5.7). Returns what it made readable.
    pub(super) fn receive_group_state(
        &mut self,
        index: usize,
        message: &Message,
        now: u32,
        events: &mut Vec<Event>,
    ) -> Result<Vec<Handled>> {
        let Ok(state) = GroupState::from_bytes(&message.body) else {
            return Ok(Vec::new());
        };
        let contact = &self.contacts[index];
        let sender = *contact.card.dh_key().as_bytes();
        let me = self.identity.dh_public();
        // The admin cannot leave, so it is in every state it sends.
        if !has(&state.members, &sender) {
            return Ok(Vec::new());
        }
        let known = self.store.group_by_group_id(&state.id)?;
        match &known {
            None if contact.hidden || contact.request || !has(&state.members, me.as_bytes()) => {
                return Ok(Vec::new());
            }
            Some(group) if group.admin != sender || state.version <= group.version => {
                return Ok(Vec::new());
            }
            _ => {}
        }
        let (id, reopened) = self.adopt_state(&state, &sender, known.as_ref(), now)?;
        events.push(Event::GroupChanged {
            group: self.group_view(&self.stored_group(id)?)?,
        });
        Ok(reopened)
    }

    /// A member's chain (§5.7). Kept even before we know the group or that
    /// they are in it, since pair bundles can overtake the admin's state.
    pub(super) fn receive_sender_key(
        &mut self,
        index: usize,
        message: &Message,
        now: u32,
    ) -> Result<Vec<Handled>> {
        let Ok(distribution) = Distribution::from_bytes(&message.body) else {
            return Ok(Vec::new());
        };
        let sender = *self.contacts[index].card.dh_key().as_bytes();
        let chain = ReceiverChain::new(&distribution);
        let inserted = self.store.insert_receiver_chain(
            &distribution.group,
            &sender,
            &chain,
            message.created_at,
            now,
        )?;
        if inserted.is_none() {
            return Ok(Vec::new());
        }
        self.forget_tags();
        self.open_carried_groups(now)
    }

    /// A member leaves one of our groups.
    pub(super) fn receive_leave(
        &mut self,
        index: usize,
        message: &Message,
        now: u32,
        events: &mut Vec<Event>,
    ) -> Result<()> {
        let Ok(group_id) = <[u8; 16]>::try_from(message.body.as_slice()) else {
            return Ok(());
        };
        let Some(group) = self.store.group_by_group_id(&GroupId(group_id))? else {
            return Ok(());
        };
        let leaver = *self.contacts[index].card.dh_key().as_bytes();
        if group.admin != *self.identity.dh_public().as_bytes()
            || !group.member
            || !group.has_member(&leaver)
        {
            return Ok(());
        }
        let members = group
            .members
            .iter()
            .filter(|card| card.dh_key().as_bytes() != &leaver)
            .cloned()
            .collect();
        let view = self.change_members(&group, members, now)?;
        events.push(Event::GroupChanged { group: view });
        Ok(())
    }

    /// Opens a group bundle on a chain we hold, if its sender is a member.
    /// A bundle we already carry is not stored or passed on again.
    pub(super) fn receive_group(
        &mut self,
        bundle: &Bundle,
        now: u32,
        carried: bool,
    ) -> Result<Option<Handled>> {
        self.refresh_tags(now);
        let Some(&chain_id) = self.group_tags.get(&bundle.tag) else {
            return Ok(None);
        };
        let Some((group_id, sender, chain)) = self.store.receiver_chain(chain_id)? else {
            return Ok(None);
        };
        let Some(group) = self.store.group_by_group_id(&group_id)? else {
            return Ok(None);
        };
        if !group.member || !group.has_member(&sender) {
            return Ok(None);
        }
        let store = &self.store;
        let Ok(opened) = chain.open(bundle, |iteration| {
            store.group_skipped_key(chain_id, iteration).ok().flatten()
        }) else {
            return Ok(None);
        };
        self.store.save_chain(chain_id, &opened.chain.to_bytes())?;
        if let Some(iteration) = opened.used {
            self.store.delete_group_skipped_key(chain_id, iteration)?;
        }
        self.store
            .store_group_skipped_keys(chain_id, &opened.skipped, now)?;

        let mut events = Vec::new();
        if opened.message.kind == Kind::Text {
            let body = String::from_utf8_lossy(&opened.message.body).into_owned();
            if let Some(id) = self.store.insert_group_message(
                group.id,
                Some(&sender),
                bundle.id,
                &body,
                opened.message.created_at,
                Some(now),
            )? {
                events.push(Event::GroupMessageReceived {
                    message: GroupMessageView {
                        id,
                        group_id: group.id,
                        outgoing: false,
                        sender: Some(self.member_name(&group, &sender)),
                        body,
                        created_at: opened.message.created_at,
                        read: false,
                    },
                });
            }
        }
        // Members carry a group bundle on for one another (§5.7).
        let relay = if carried {
            Vec::new()
        } else {
            self.store.insert_bundle(bundle, false, now)?;
            forwarded(bundle).into_iter().collect()
        };
        Ok(Some(Handled {
            outcome: Outcome::Received,
            relay,
            events,
        }))
    }

    /// Opens the carried bundles that a chain or member we just learned of
    /// makes readable.
    pub(super) fn open_carried_groups(&mut self, now: u32) -> Result<Vec<Handled>> {
        self.refresh_tags(now);
        let mut opened = Vec::new();
        for bundle in self.store.carried_bundles(now)? {
            if self.group_tags.contains_key(&bundle.tag)
                && !self.store.has_group_message(bundle.id)?
            {
                opened.extend(self.receive_group(&bundle, now, true)?);
            }
        }
        Ok(opened)
    }

    /// Records a group state and does what it asks of us: meet the members
    /// we do not know, start a chain or hand ours on, or forget the group.
    fn adopt_state(
        &mut self,
        state: &GroupState,
        admin: &[u8; 32],
        before: Option<&StoredGroup>,
        now: u32,
    ) -> Result<(i64, Vec<Handled>)> {
        let me = *self.identity.dh_public().as_bytes();
        let member = has(&state.members, &me);
        let id = self.store.save_group(state, admin, member, now)?;
        let was_member = before.is_some_and(|group| group.member);
        if !member {
            if was_member {
                self.store.delete_group_chains(&state.id)?;
                self.forget_tags();
            }
            return Ok((id, Vec::new()));
        }

        let mut new_peers = Vec::new();
        for card in state
            .members
            .iter()
            .filter(|card| card.dh_key().as_bytes() != &me)
        {
            match self.peer_index(card.dh_key().as_bytes()) {
                // The state gave them our card.
                Ok(index) => self.mark_introduced(index)?,
                Err(_) => {
                    let peer_id = self.store.insert_peer(card, now)?;
                    let keys = PairKeys::derive(&self.identity, card.dh_key())?;
                    let session = Session::new(&self.identity, card.dh_key(), &keys);
                    self.contacts.push(Contact {
                        id: peer_id,
                        card: card.clone(),
                        keys,
                        request: false,
                        introduced: true,
                        hidden: true,
                        session,
                    });
                    new_peers.push(peer_id);
                }
            }
        }
        self.forget_tags();

        // New members get our chain as it stands; after a removal everyone
        // gets a new one, so the removed member reads nothing more.
        let old: Vec<[u8; 32]> = before
            .filter(|_| was_member)
            .map(|group| keys_of(&group.members))
            .unwrap_or_default();
        let others: Vec<[u8; 32]> = keys_of(&state.members)
            .into_iter()
            .filter(|key| key != &me)
            .collect();
        let removed = old.iter().any(|key| !others.contains(key) && key != &me);
        let (chain, recipients) = match self.store.own_chain(&state.id)? {
            Some((_, chain)) if was_member && !removed => {
                let added = others
                    .into_iter()
                    .filter(|key| !old.contains(key))
                    .collect();
                (chain, added)
            }
            _ => {
                let chain = SenderChain::generate(&mut OsRng);
                self.store.insert_own_chain(&state.id, &me, &chain, now)?;
                (chain, others)
            }
        };
        let message = Message::sender_key(&chain.distribution(state.id), now);
        for key in recipients {
            let index = self.peer_index(&key)?;
            self.send_pair(index, &message, now)?;
        }

        let mut reopened = Vec::new();
        for peer in new_peers {
            reopened.extend(self.open_carried_from(peer, now)?);
        }
        reopened.extend(self.open_carried_groups(now)?);
        Ok((id, reopened))
    }

    /// As admin: records a new member list and sends it to the members, old
    /// and new, the removed ones included.
    fn change_members(
        &mut self,
        group: &StoredGroup,
        members: Vec<ContactCard>,
        now: u32,
    ) -> Result<GroupView> {
        let state = GroupState {
            id: group.group_id,
            version: group.version + 1,
            name: group.name.clone(),
            members,
        };
        self.publish_state(&state, &group.members, now)?;
        self.group_view(&self.stored_group(group.id)?)
    }

    /// As admin: adopts a state and sends it to its members and to `before`.
    fn publish_state(
        &mut self,
        state: &GroupState,
        before: &[ContactCard],
        now: u32,
    ) -> Result<i64> {
        let me = *self.identity.dh_public().as_bytes();
        let known = self.store.group_by_group_id(&state.id)?;
        let (id, _) = self.adopt_state(state, &me, known.as_ref(), now)?;
        let message = Message::group(state, now)?;
        let mut sent = vec![me];
        for key in keys_of(&state.members).into_iter().chain(keys_of(before)) {
            if !sent.contains(&key) {
                sent.push(key);
                let index = self.peer_index(&key)?;
                self.send_pair(index, &message, now)?;
            }
        }
        Ok(id)
    }

    /// Seals a pair message and queues it to go out on every link.
    fn send_pair(&mut self, index: usize, message: &Message, now: u32) -> Result<()> {
        let bundle = self.seal_pair(index, message, now)?;
        self.store.insert_bundle(&bundle, true, now)?;
        self.own_out
            .extend(forwarded(&bundle).map(|frame| (frame, bundle.tag)));
        Ok(())
    }

    /// The card of a contact who can join a group: accepted, and holding
    /// our card, so it can open what we send.
    fn joinable_card(&self, contact_id: i64) -> Result<ContactCard> {
        let contact = &self.contacts[self.contact_index(contact_id)?];
        if contact.hidden || contact.request {
            return Err(Error::Invalid("unknown contact"));
        }
        if !contact.introduced {
            return Err(Error::Invalid(
                "write to them once first: they need your card to join a group",
            ));
        }
        Ok(contact.card.clone())
    }

    fn peer_index(&self, key: &[u8; 32]) -> Result<usize> {
        self.contacts
            .iter()
            .position(|contact| contact.card.dh_key().as_bytes() == key)
            .ok_or(Error::Invalid("unknown member"))
    }

    fn stored_group(&self, group_id: i64) -> Result<StoredGroup> {
        self.store
            .group(group_id)?
            .ok_or(Error::Invalid("unknown group"))
    }

    fn admin_group(&self, group_id: i64) -> Result<StoredGroup> {
        let group = self.stored_group(group_id)?;
        if group.admin != *self.identity.dh_public().as_bytes() || !group.member {
            return Err(Error::Invalid(
                "only the group's admin can change its members",
            ));
        }
        Ok(group)
    }

    pub(super) fn group_view(&self, group: &StoredGroup) -> Result<GroupView> {
        let me = self.identity.dh_public();
        let last = self.store.last_group_message(group.id)?;
        Ok(GroupView {
            id: group.id,
            name: group.name.clone(),
            admin: group.admin == *me.as_bytes(),
            member: group.member,
            members: group
                .members
                .iter()
                .map(|card| {
                    let key = card.dh_key().as_bytes();
                    let contact = self
                        .contacts
                        .iter()
                        .find(|c| c.card.dh_key() == card.dh_key());
                    MemberView {
                        name: self.member_name(group, key),
                        fingerprint: card.fingerprint().to_string(),
                        me: key == me.as_bytes(),
                        admin: key == &group.admin,
                        contact_id: contact
                            .filter(|contact| !contact.hidden && !contact.request)
                            .map(|contact| contact.id),
                    }
                })
                .collect(),
            last_message: last.as_ref().map(|(body, _)| body.clone()),
            last_at: last.map(|(_, at)| at),
            unread: self.store.group_unread_count(group.id)?,
        })
    }

    fn group_message_view(
        &self,
        group: &StoredGroup,
        message: StoredGroupMessage,
    ) -> GroupMessageView {
        GroupMessageView {
            id: message.id,
            group_id: group.id,
            outgoing: message.sender.is_none(),
            sender: message.sender.map(|key| self.member_name(group, &key)),
            body: message.body,
            created_at: message.created_at,
            read: message.read,
        }
    }

    /// A member's name: ours, as we know them as a contact, or as the
    /// group's state gives it.
    fn member_name(&self, group: &StoredGroup, key: &[u8; 32]) -> String {
        if key == self.identity.dh_public().as_bytes() {
            return self.identity.name().to_owned();
        }
        self.contacts
            .iter()
            .map(|contact| &contact.card)
            .chain(&group.members)
            .find(|card| card.dh_key().as_bytes() == key)
            .map_or_else(|| "?".to_owned(), |card| card.name().to_owned())
    }
}

fn has(members: &[ContactCard], key: &[u8; 32]) -> bool {
    members.iter().any(|card| card.dh_key().as_bytes() == key)
}

fn keys_of(members: &[ContactCard]) -> Vec<[u8; 32]> {
    members
        .iter()
        .map(|card| *card.dh_key().as_bytes())
        .collect()
}

fn check_size(members: &[ContactCard]) -> Result<()> {
    if members.len() > MAX_MEMBERS {
        return Err(Error::Invalid("a group has at most 16 members"));
    }
    Ok(())
}

//! What a node does with bundles and tombstones, independent of any link.
//! Every method takes the current time, so tests can control it.

use std::collections::HashMap;

use carrychat_proto::bundle::{
    open, seal, Bundle, BundleId, Kind, Message, Tombstone, DEFAULT_HOP_LIMIT, DEFAULT_TTL_SECS,
    MAX_READ_IDS, MAX_TTL_SECS,
};
use carrychat_proto::frame::{Frame, Reassembler};
use carrychat_proto::identity::{ContactCard, Identity};
use carrychat_proto::intro::{mailbox_tag, open_introduction, seal_introduction};
use carrychat_proto::pair::{PairKeys, RecipientTag, TAG_WINDOW_FUTURE_DAYS, TAG_WINDOW_PAST_DAYS};
use carrychat_proto::ratchet::{Opened, Role, Session};
use carrychat_proto::{day_of, DAY_SECS};
use rand_core::OsRng;

use crate::journey::{Journey, Via};
use crate::store::{FileTable, Store};
use crate::{ContactView, Error, Event, MessageView, Profile, Result, Settings};

mod channels;
mod files;
mod groups;

/// Bundles that claim to live longer than this from now are refused (§8).
const MAX_FUTURE_EXPIRY_SECS: u32 = MAX_TTL_SECS + DAY_SECS;

/// What happened to an incoming frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A message for us.
    Received,
    /// Someone else's bundle, now carried.
    Carried,
    /// A tombstone for one of our messages.
    Delivered,
    /// A tombstone for a bundle we carried or had seen.
    Tombstoned,
    /// Part of a fragmented bundle.
    Fragment,
    Duplicate,
    Dropped,
}

pub struct Handled {
    pub outcome: Outcome,
    /// Frames to pass on to other links.
    pub relay: Vec<Frame>,
    pub events: Vec<Event>,
}

impl Handled {
    fn quiet(outcome: Outcome) -> Self {
        Self {
            outcome,
            relay: Vec::new(),
            events: Vec::new(),
        }
    }
}

struct Contact {
    id: i64,
    card: ContactCard,
    keys: PairKeys,
    /// Wrote to us first and is waiting to be accepted (§5.5).
    request: bool,
    /// Has our card, so we can send pair bundles instead of introductions.
    introduced: bool,
    /// Known only through a group (§5.7): not listed among the contacts.
    hidden: bool,
    session: Session,
}

pub struct Engine {
    store: Store,
    identity: Identity,
    contacts: Vec<Contact>,
    /// Recipient tags of bundles for us, for the days in the tag window.
    tags: HashMap<RecipientTag, usize>,
    tags_day: Option<u32>,
    /// Tags of group bundles, by the row of the chain they belong to.
    group_tags: HashMap<RecipientTag, i64>,
    /// Tags of channel posts, by channel.
    channel_tags: HashMap<RecipientTag, i64>,
    /// Tags of the chunks of files still arriving, by file and place.
    file_tags: HashMap<RecipientTag, (i64, u32)>,
    /// Whether the tags changed since the node last renewed its subscription.
    tags_changed: bool,
    reassembler: Reassembler,
    /// Own bundles made while handling frames, such as session starts.
    own_out: Vec<(Frame, RecipientTag)>,
}

impl Engine {
    pub fn create(store: Store, name: &str) -> Result<Self> {
        let identity = Identity::generate(name)?;
        let (signing, dh) = identity.secret_bytes();
        store.set_meta("signing_secret", &signing)?;
        store.set_meta("dh_secret", &dh)?;
        store.set_meta("name", name.as_bytes())?;
        Self::with_identity(store, identity)
    }

    /// Loads the node in `store`, or returns `None` if it has no identity yet.
    pub fn load(store: Store) -> Result<Option<Self>> {
        let (Some(signing), Some(dh), Some(name)) = (
            store.meta("signing_secret")?,
            store.meta("dh_secret")?,
            store.meta("name")?,
        ) else {
            return Ok(None);
        };
        let secret = |bytes: Vec<u8>| -> Result<[u8; 32]> {
            bytes
                .try_into()
                .map_err(|_| Error::Invalid("stored secret key is corrupt"))
        };
        let name = String::from_utf8(name).map_err(|_| Error::Invalid("stored name is corrupt"))?;
        let identity = Identity::from_secret_bytes(&secret(signing)?, &secret(dh)?, &name)?;
        Self::with_identity(store, identity).map(Some)
    }

    fn with_identity(store: Store, identity: Identity) -> Result<Self> {
        let contacts = load_contacts(&store, &identity)?;
        Ok(Self {
            store,
            identity,
            contacts,
            tags: HashMap::new(),
            tags_day: None,
            group_tags: HashMap::new(),
            channel_tags: HashMap::new(),
            file_tags: HashMap::new(),
            tags_changed: false,
            reassembler: Reassembler::default(),
            own_out: Vec::new(),
        })
    }

    /// Runs a change atomically: if it fails, the store and what the engine
    /// keeps in memory are as they were before, so nothing is half done, such
    /// as a receipt sent for a message that was not kept.
    pub fn atomically<T>(&mut self, change: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let own_out = self.own_out.len();
        self.store.begin()?;
        let result = change(self).and_then(|value| {
            self.store.commit()?;
            Ok(value)
        });
        if result.is_err() {
            self.store.rollback();
            self.own_out.truncate(own_out);
            // The store is as before; take contacts, sessions and name from it.
            if let Ok(contacts) = load_contacts(&self.store, &self.identity) {
                self.contacts = contacts;
            }
            if let Ok(Some(name)) = self.store.meta("name") {
                let (signing, dh) = self.identity.secret_bytes();
                if let Ok(identity) =
                    Identity::from_secret_bytes(&signing, &dh, &String::from_utf8_lossy(&name))
                {
                    self.identity = identity;
                }
            }
            self.forget_tags();
        }
        result
    }

    pub fn profile(&self) -> Profile {
        Profile {
            name: self.identity.name().to_owned(),
            fingerprint: self.identity.fingerprint().to_string(),
            card: self.identity.card().to_text(),
        }
    }

    /// Adds a contact from its card text, or accepts it if it was a request.
    /// Bundles from them that this node was already carrying are opened right
    /// away; their results are returned.
    pub fn add_contact(
        &mut self,
        card_text: &str,
        now: u32,
    ) -> Result<(ContactView, Vec<Handled>)> {
        let card = ContactCard::from_text(card_text)?;
        if card.dh_key() == &self.identity.dh_public() {
            return Err(Error::Invalid("that is your own card"));
        }
        self.store.unblock(card.dh_key().as_bytes())?;
        let id = self.store.insert_contact(&card, now, false)?;
        if let Some(existing) = self.contacts.iter_mut().find(|contact| contact.id == id) {
            existing.card = card;
            existing.request = false;
            existing.hidden = false;
        } else {
            let keys = PairKeys::derive(&self.identity, card.dh_key())?;
            let session = Session::new(&self.identity, card.dh_key(), &keys);
            self.contacts.push(Contact {
                id,
                card,
                keys,
                request: false,
                introduced: false,
                hidden: false,
                session,
            });
        }
        self.forget_tags();
        let opened = self.open_carried_from(id, now)?;
        Ok((self.contact_view(id)?, opened))
    }

    /// Accepts a request: the sender becomes a contact and can be answered.
    pub fn accept_contact(&mut self, contact_id: i64) -> Result<ContactView> {
        let index = self.contact_index(contact_id)?;
        self.store.accept_contact(contact_id)?;
        self.contacts[index].request = false;
        self.contact_view(contact_id)
    }

    /// Declines a request: deletes it with its messages and drops later
    /// introductions from the same sender.
    pub fn decline_contact(&mut self, contact_id: i64) -> Result<()> {
        let index = self.contact_index(contact_id)?;
        if !self.contacts[index].request {
            return Err(Error::Invalid("only requests can be declined"));
        }
        let contact = self.contacts.remove(index);
        self.store.block(contact.card.dh_key().as_bytes())?;
        self.store.delete_contact(contact_id)?;
        self.forget_tags();
        Ok(())
    }

    pub fn contacts(&self) -> Result<Vec<ContactView>> {
        self.contacts
            .iter()
            .filter(|contact| !contact.hidden)
            .map(|contact| self.contact_view(contact.id))
            .collect()
    }

    pub fn messages(&self, contact_id: i64) -> Result<Vec<MessageView>> {
        self.contact_index(contact_id)?;
        let mut messages = self.store.messages(contact_id)?;
        for message in &mut messages {
            message.file = self.attached(FileTable::Messages, message.id)?;
        }
        Ok(messages)
    }

    /// Notes where one of our messages went; see [`Store::record_handoff`].
    pub fn record_handoff(
        &self,
        id: BundleId,
        via: Via,
        peer: u64,
        now: u32,
    ) -> Result<Option<(i64, i64)>> {
        self.store.record_handoff(id, via, peer, now)
    }

    pub fn journey(&self, message_id: i64) -> Result<Journey> {
        self.store
            .journey(message_id)?
            .ok_or(Error::Invalid("unknown message"))
    }

    /// Seals a text message. The returned frame is ready to send on any link.
    pub fn send_text(
        &mut self,
        contact_id: i64,
        text: &str,
        now: u32,
    ) -> Result<(MessageView, Frame)> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::Invalid("message is empty"));
        }
        let index = self.contact_index(contact_id)?;
        let contact = &self.contacts[index];
        if contact.request {
            return Err(Error::Invalid("accept the request before you reply"));
        }
        let expires_at = now + DEFAULT_TTL_SECS;
        // Until the contact has our card, every message carries it (§5.5).
        let bundle = if contact.introduced {
            self.seal_pair(index, &Message::text(text, now), now)?
        } else {
            seal_introduction(
                &self.identity,
                contact.card.dh_key(),
                text,
                now,
                expires_at,
                DEFAULT_HOP_LIMIT,
            )?
        };
        self.store.insert_bundle(&bundle, true, now)?;
        let message = self
            .store
            .insert_message(contact_id, true, bundle.id, text, now, None)?
            .ok_or(Error::Invalid("message already exists"))?;
        let frame = forwarded(&bundle).expect("a new bundle has hops left");
        Ok((message, frame))
    }

    pub fn handle(&mut self, frame: Frame, now: u32) -> Result<Handled> {
        self.handle_with(frame, now, true)
    }

    /// Handles a frame. With `carry` false, bundles that are not for us are
    /// dropped instead of carried, for links where anyone can inject frames.
    pub fn handle_with(&mut self, frame: Frame, now: u32, carry: bool) -> Result<Handled> {
        match frame {
            Frame::Bundle(bundle) => self.handle_bundle(bundle, now, carry),
            Frame::Tombstone {
                hop_limit,
                tombstone,
            } => self.handle_tombstone(hop_limit, tombstone, now),
            Frame::Fragment(fragment) => match self.reassembler.push(fragment, now) {
                Ok(Some(bundle)) => self.handle_bundle(bundle, now, carry),
                Ok(None) => Ok(Handled::quiet(Outcome::Fragment)),
                Err(_) => Ok(Handled::quiet(Outcome::Dropped)),
            },
            Frame::Chunk(bundle) => self.handle_chunk(bundle, now, carry),
        }
    }

    /// Ids of the bundles this node can pass on, for a sync offer.
    pub fn offer(&self, now: u32) -> Result<Vec<BundleId>> {
        Ok(self
            .store
            .forwardable_bundles(now)?
            .iter()
            .map(|bundle| bundle.id)
            .collect())
    }

    /// The ids in an offer this node has never seen.
    pub fn missing(&self, ids: &[BundleId]) -> Result<Vec<BundleId>> {
        let mut missing = Vec::new();
        for &id in ids {
            if !self.seen(id)? {
                missing.push(id);
            }
        }
        Ok(missing)
    }

    /// The bundles a peer asked for, ready to send.
    pub fn frames_for(&self, ids: &[BundleId], now: u32) -> Result<Vec<Frame>> {
        let mut frames = Vec::new();
        for &id in ids {
            if let Some(bundle) = self.store.bundle(id)? {
                if !bundle.is_expired(now) {
                    frames.extend(self.passed_on(&bundle)?);
                }
            }
        }
        Ok(frames)
    }

    pub fn tombstone_frames(&self) -> Result<Vec<Frame>> {
        Ok(self
            .store
            .tombstones()?
            .into_iter()
            .map(|tombstone| Frame::Tombstone {
                hop_limit: DEFAULT_HOP_LIMIT,
                tombstone,
            })
            .collect())
    }

    /// Everything worth putting on a USB stick: bundles, then tombstones.
    pub fn export_frames(&self, now: u32) -> Result<Vec<Frame>> {
        let mut frames = Vec::new();
        for bundle in self.store.forwardable_bundles(now)? {
            frames.extend(self.passed_on(&bundle)?);
        }
        frames.extend(self.tombstone_frames()?);
        Ok(frames)
    }

    /// What to ask internet relays for (§9.3): the tags of bundles to us from
    /// every contact and our mailbox tags over the tag window, and the ids of
    /// our own bundles still waiting for a receipt.
    pub fn subscription(&mut self, now: u32) -> Result<(Vec<RecipientTag>, Vec<BundleId>)> {
        self.refresh_tags(now);
        let me = self.identity.dh_public();
        let mut tags: Vec<RecipientTag> = self.tags.keys().copied().collect();
        tags.extend(self.group_tags.keys());
        tags.extend(self.channel_tags.keys());
        tags.extend(self.file_tags.keys());
        tags.extend(tag_window(now).map(|day| mailbox_tag(&me, day)));
        let waiting = self
            .store
            .own_bundles(now)?
            .iter()
            .map(|bundle| bundle.id)
            .collect();
        Ok((tags, waiting))
    }

    /// Own bundles waiting for a receipt, ready to post, with their tags.
    pub fn outbox(&self, now: u32) -> Result<Vec<(Frame, RecipientTag)>> {
        let mut outbox = Vec::new();
        for bundle in self.store.own_bundles(now)? {
            let tag = self.publish_tag(&bundle)?;
            outbox.extend(self.passed_on(&bundle)?.map(|frame| (frame, tag)));
        }
        Ok(outbox)
    }

    /// The tag a bundle is posted under where a tag is needed to find it
    /// (§9.3): its own for pair bundles, the recipient's mailbox tag for our
    /// introductions.
    pub fn publish_tag(&self, bundle: &Bundle) -> Result<RecipientTag> {
        let Some((contact_id, created_at)) = self.store.outgoing_message(bundle.id)? else {
            return Ok(bundle.tag);
        };
        let Ok(index) = self.contact_index(contact_id) else {
            return Ok(bundle.tag);
        };
        let contact = &self.contacts[index];
        let day = day_of(created_at);
        let recipient = contact.card.dh_key();
        if contact.keys.recipient_tag(recipient, day) == bundle.tag {
            Ok(bundle.tag)
        } else {
            Ok(mailbox_tag(recipient, day))
        }
    }

    /// Carried bundles that no internet relay has taken yet, for a node that
    /// just reached one: it may have taken them over the local network or
    /// from a file while it had no internet, or a relay may have refused
    /// them. They count as uploaded once a relay accepts them.
    pub fn uploads(&self, now: u32) -> Result<Vec<(Frame, RecipientTag)>> {
        let mut uploads = Vec::new();
        for bundle in self.store.bundles_to_upload(now)? {
            uploads.extend(self.passed_on(&bundle)?.map(|frame| (frame, bundle.tag)));
        }
        Ok(uploads)
    }

    /// Notes that an internet relay accepted a carried bundle.
    pub fn mark_uploaded(&self, id: BundleId) -> Result<()> {
        self.store.mark_uploaded(id)
    }

    /// What a link that comes up later should carry from us: our own
    /// bundles still waiting for a receipt, then the receipts for the
    /// `receipts` latest bundles delivered to us.
    pub fn pending_own(&self, now: u32, receipts: usize) -> Result<Vec<Frame>> {
        let mut frames = Vec::new();
        for bundle in self.store.own_bundles(now)? {
            frames.extend(self.passed_on(&bundle)?);
        }
        frames.extend(
            self.store
                .received_tombstones(receipts)?
                .into_iter()
                .map(|tombstone| Frame::Tombstone {
                    hop_limit: DEFAULT_HOP_LIMIT,
                    tombstone,
                }),
        );
        Ok(frames)
    }

    /// Tombstones of the latest bundles delivered to us, to post again.
    pub fn received_tombstones(&self) -> Result<Vec<Frame>> {
        Ok(self
            .store
            .received_tombstones(500)?
            .into_iter()
            .map(|tombstone| Frame::Tombstone {
                hop_limit: DEFAULT_HOP_LIMIT,
                tombstone,
            })
            .collect())
    }

    /// The secret that one-time Nostr keys are derived from (§9.3), created
    /// on first use.
    pub fn nostr_secret(&self) -> Result<[u8; 32]> {
        if let Some(secret) = self.store.meta("nostr_secret")? {
            return secret
                .try_into()
                .map_err(|_| Error::Invalid("stored Nostr secret is corrupt"));
        }
        let mut secret = [0; 32];
        rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut secret);
        self.store.set_meta("nostr_secret", &secret)?;
        Ok(secret)
    }

    /// Own messages still waiting for delivery, and bundles carried for others.
    pub fn counts(&self) -> Result<(usize, usize)> {
        self.store.counts()
    }

    /// Marks a contact's messages read. Returns the read receipts to send,
    /// unless read receipts are off or the contact is a request.
    pub fn mark_read(&mut self, contact_id: i64, now: u32) -> Result<Vec<(Frame, RecipientTag)>> {
        let index = self.contact_index(contact_id)?;
        let ids = self.store.mark_incoming_read(contact_id, now)?;
        let contact = &self.contacts[index];
        if ids.is_empty()
            || contact.request
            || !contact.introduced
            || !self.settings()?.read_receipts
        {
            return Ok(Vec::new());
        }
        let mut receipts = Vec::new();
        for chunk in ids.chunks(MAX_READ_IDS) {
            let bundle = self.seal_pair(index, &Message::read(chunk, now), now)?;
            self.store.insert_bundle(&bundle, true, now)?;
            receipts.extend(forwarded(&bundle).map(|frame| (frame, bundle.tag)));
        }
        Ok(receipts)
    }

    /// Changes our name. Contacts that have our card get the new one; the
    /// others get it with our next introduction.
    pub fn rename(
        &mut self,
        name: &str,
        now: u32,
    ) -> Result<(Profile, Vec<(Frame, RecipientTag)>)> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("choose a name"));
        }
        let (signing, dh) = self.identity.secret_bytes();
        self.identity = Identity::from_secret_bytes(&signing, &dh, name)?;
        self.store.set_meta("name", name.as_bytes())?;
        let card = self.identity.card();
        let mut updates = Vec::new();
        let targets: Vec<usize> = (0..self.contacts.len())
            .filter(|&index| !self.contacts[index].request && self.contacts[index].introduced)
            .collect();
        for index in targets {
            let bundle = self.seal_pair(index, &Message::card(&card, now), now)?;
            self.store.insert_bundle(&bundle, true, now)?;
            updates.extend(forwarded(&bundle).map(|frame| (frame, bundle.tag)));
        }
        Ok((self.profile(), updates))
    }

    pub fn settings(&self) -> Result<Settings> {
        Ok(Settings {
            read_receipts: self.store.meta("read_receipts")?.as_deref() != Some(b"0".as_slice()),
            tor: self.store.meta("tor")?.as_deref() == Some(b"1".as_slice()),
            bluetooth: self.store.meta("bluetooth")?.as_deref() != Some(b"0".as_slice()),
            nearby: self.store.meta("nearby")?.as_deref() == Some(b"1".as_slice()),
            lora_device: self
                .store
                .meta("lora_device")?
                .and_then(|device| String::from_utf8(device).ok())
                .filter(|device| !device.is_empty()),
            reticulum: self
                .store
                .meta("reticulum")?
                .and_then(|address| String::from_utf8(address).ok())
                .filter(|address| !address.is_empty()),
        })
    }

    /// The Reticulum node to connect to, as `host:port`, or none.
    pub fn set_reticulum(&self, address: Option<&str>) -> Result<()> {
        self.store
            .set_meta("reticulum", address.unwrap_or_default().as_bytes())
    }

    /// The CarryChat destinations heard on Reticulum, as `Reticulum` keeps
    /// them, for asking for their paths after a restart.
    pub fn reticulum_peers(&self) -> Result<Vec<u8>> {
        Ok(self.store.meta("reticulum_peers")?.unwrap_or_default())
    }

    pub fn set_reticulum_peers(&self, peers: &[u8]) -> Result<()> {
        self.store.set_meta("reticulum_peers", peers)
    }

    /// This node's Reticulum identity: its X25519 and Ed25519 secret keys,
    /// made the first time they are asked for. They have nothing to do with
    /// the CarryChat identity, so the Reticulum network cannot tell whose
    /// node it is.
    pub fn reticulum_secret(&self) -> Result<[u8; 64]> {
        if let Some(secret) = self.store.meta("reticulum_identity")? {
            if let Ok(secret) = secret.try_into() {
                return Ok(secret);
            }
        }
        let mut secret = [0; 64];
        rand_core::RngCore::fill_bytes(&mut OsRng, &mut secret);
        self.store.set_meta("reticulum_identity", &secret)?;
        Ok(secret)
    }

    /// The Meshtastic or MeshCore device to use for LoRa, as `LoraDevice`
    /// writes it, or none.
    pub fn set_lora_device(&self, device: Option<&str>) -> Result<()> {
        self.store
            .set_meta("lora_device", device.unwrap_or_default().as_bytes())
    }

    /// Whether people nearby who are visible too can see us.
    pub fn set_nearby(&self, on: bool) -> Result<()> {
        self.store.set_meta("nearby", if on { b"1" } else { b"0" })
    }

    /// Our own card, as contacts get it.
    pub fn card(&self) -> ContactCard {
        self.identity.card()
    }

    /// The id of the listed contact with this card's key, if there is one.
    pub fn contact_id_of(&self, card: &ContactCard) -> Option<i64> {
        self.contacts
            .iter()
            .find(|contact| !contact.hidden && contact.card.dh_key() == card.dh_key())
            .map(|contact| contact.id)
    }

    /// Whether the node uses Bluetooth LE, from its next start.
    pub fn set_bluetooth(&self, on: bool) -> Result<()> {
        self.store
            .set_meta("bluetooth", if on { b"1" } else { b"0" })
    }

    /// Whether the internet link should go over Tor, from its next start.
    pub fn set_tor(&self, on: bool) -> Result<()> {
        self.store.set_meta("tor", if on { b"1" } else { b"0" })
    }

    pub fn set_read_receipts(&self, on: bool) -> Result<()> {
        self.store
            .set_meta("read_receipts", if on { b"1" } else { b"0" })
    }

    pub fn prune(&self, now: u32) -> Result<()> {
        self.store.prune(now)
    }

    fn handle_bundle(&mut self, bundle: Bundle, now: u32, carry: bool) -> Result<Handled> {
        if bundle.is_expired(now) || bundle.expires_at > now.saturating_add(MAX_FUTURE_EXPIRY_SECS)
        {
            return Ok(Handled::quiet(Outcome::Dropped));
        }
        if self.seen(bundle.id)? {
            return Ok(Handled::quiet(Outcome::Duplicate));
        }
        if let Some(contact) = self.contact_for_tag(&bundle.tag, now) {
            if let Some(handled) = self.deliver(contact, &bundle, now)? {
                return Ok(handled);
            }
        }
        if let Some(handled) = self.receive_group(&bundle, now, false)? {
            return Ok(handled);
        }
        if let Some(handled) = self.receive_post(&bundle, now, false)? {
            return Ok(handled);
        }
        if let Some(handled) = self.introduce(&bundle, now)? {
            return Ok(handled);
        }
        if !carry {
            return Ok(Handled::quiet(Outcome::Dropped));
        }
        self.store.insert_bundle(&bundle, false, now)?;
        Ok(Handled {
            outcome: Outcome::Carried,
            relay: forwarded(&bundle).into_iter().collect(),
            events: Vec::new(),
        })
    }

    /// Opens a bundle for us and records it. Returns `None` if it does not
    /// open, in which case it is carried like anybody else's (§5.3).
    fn deliver(&mut self, index: usize, bundle: &Bundle, now: u32) -> Result<Option<Handled>> {
        let was_secure = self.contacts[index].session.is_established();
        let Some((message, with_pair_key)) = self.open_pair(index, bundle, now)? else {
            return Ok(None);
        };
        self.mark_introduced(index)?;
        let contact_id = self.contacts[index].id;
        let tombstone = Tombstone::for_bundle(&self.contacts[index].keys, bundle);
        self.store.insert_tombstone(&tombstone, bundle.expires_at)?;

        let mut events = Vec::new();
        let mut reopened = Vec::new();
        match message.kind {
            Kind::Text => {
                // A group peer that writes to us directly becomes a request.
                let revealed = self.contacts[index].hidden;
                if revealed {
                    self.store.reveal_peer(contact_id)?;
                    self.contacts[index].hidden = false;
                    self.contacts[index].request = true;
                }
                let body = String::from_utf8_lossy(&message.body);
                if let Some(view) = self.store.insert_message(
                    contact_id,
                    false,
                    bundle.id,
                    &body,
                    message.created_at,
                    Some(now),
                )? {
                    events.push(Event::MessageReceived { message: view });
                }
                if revealed {
                    events.push(Event::ContactRequest {
                        contact: self.contact_view(contact_id)?,
                    });
                }
            }
            Kind::Read => {
                let message_ids =
                    self.store
                        .mark_read_by_peer(contact_id, &message.read_ids(), now)?;
                if !message_ids.is_empty() {
                    events.push(Event::MessagesRead {
                        contact_id,
                        message_ids,
                    });
                }
            }
            Kind::Card => {
                // A new name, but only on the same keys (§5.2).
                let current = &self.contacts[index].card;
                if let Ok(card) = ContactCard::from_bytes(&message.body) {
                    if card.signing_key() == current.signing_key()
                        && card.dh_key() == current.dh_key()
                        && &card != current
                    {
                        self.store.update_contact_card(contact_id, &card)?;
                        self.contacts[index].card = card;
                        events.push(Event::ContactChanged {
                            contact: self.contact_view(contact_id)?,
                        });
                    }
                }
            }
            Kind::File if !self.contacts[index].hidden => {
                if let Some((file, opened)) =
                    self.take_offer(&message.body, true, bundle.expires_at, now)?
                {
                    reopened = opened;
                    if let Some(mut view) = self.store.insert_message(
                        contact_id,
                        false,
                        bundle.id,
                        "",
                        message.created_at,
                        Some(now),
                    )? {
                        self.store.attach_file(FileTable::Messages, view.id, file)?;
                        view.file = self.file_view(file)?;
                        events.push(Event::MessageReceived { message: view });
                    }
                }
            }
            Kind::Group => {
                reopened = self.receive_group_state(index, &message, now, &mut events)?;
            }
            Kind::SenderKey => reopened = self.receive_sender_key(index, &message, now)?,
            Kind::Leave => self.receive_leave(index, &message, now, &mut events)?,
            Kind::File | Kind::Session | Kind::Unknown(_) => {}
        }
        // Group peers are not listed, so there is nothing to show of them.
        if self.contacts[index].session.is_established() != was_secure
            && !self.contacts[index].hidden
            && !events
                .iter()
                .any(|event| matches!(event, Event::ContactChanged { .. }))
        {
            events.push(Event::ContactChanged {
                contact: self.contact_view(contact_id)?,
            });
        }
        // A contact that writes with the pair key waits for us to start (§5.6).
        let contact = &self.contacts[index];
        if with_pair_key
            && !contact.request
            && contact.session.role() == Role::Initiator
            && !contact.session.started()
        {
            let start = self.seal_pair(index, &Message::session(now), now)?;
            self.store.insert_bundle(&start, true, now)?;
            self.own_out
                .extend(forwarded(&start).map(|frame| (frame, start.tag)));
        }
        let mut handled = Handled {
            outcome: Outcome::Received,
            relay: vec![Frame::Tombstone {
                hop_limit: DEFAULT_HOP_LIMIT,
                tombstone,
            }],
            events,
        };
        for opened in reopened {
            handled.relay.extend(opened.relay);
            handled.events.extend(opened.events);
        }
        Ok(Some(handled))
    }

    /// Seals a message to a contact: in their ratchet session when there is
    /// one, otherwise with the pair key (§5.6).
    fn seal_pair(&mut self, index: usize, message: &Message, now: u32) -> Result<Bundle> {
        let contact = &self.contacts[index];
        let expires_at = now + DEFAULT_TTL_SECS;
        let mut session = contact.session.clone();
        match session.seal(
            &contact.keys,
            contact.card.dh_key(),
            message,
            expires_at,
            DEFAULT_HOP_LIMIT,
            &mut OsRng,
        ) {
            Ok(bundle) => {
                self.store.save_session(contact.id, &session.to_bytes())?;
                self.contacts[index].session = session;
                Ok(bundle)
            }
            Err(carrychat_proto::Error::NoSession) => Ok(seal(
                &contact.keys,
                contact.card.dh_key(),
                message,
                expires_at,
                DEFAULT_HOP_LIMIT,
            )?),
            Err(error) => Err(error.into()),
        }
    }

    /// Opens a contact's pair bundle: as a ratchet message, else with the pair
    /// key. Also says whether it took the pair key.
    fn open_pair(
        &mut self,
        index: usize,
        bundle: &Bundle,
        now: u32,
    ) -> Result<Option<(Message, bool)>> {
        let me = self.identity.dh_public();
        let contact = &self.contacts[index];
        let id = contact.id;
        let store = &self.store;
        let ratchet = contact.session.open(
            &contact.keys,
            &me,
            bundle,
            |ratchet, n| store.skipped_key(id, ratchet, n).ok().flatten(),
            &mut OsRng,
        );
        match ratchet {
            Ok(Opened {
                message,
                session,
                skipped,
                used,
            }) => {
                self.store.save_session(id, &session.to_bytes())?;
                if let Some((ratchet, n)) = used {
                    self.store.delete_skipped_key(id, &ratchet, n)?;
                }
                self.store.store_skipped_keys(id, &skipped, now)?;
                self.contacts[index].session = session;
                Ok(Some((message, false)))
            }
            Err(_) => Ok(open(&contact.keys, &me, bundle)
                .ok()
                .map(|message| (message, true))),
        }
    }

    /// Own bundles made while handling frames, such as session starts, for
    /// the node to send on every link.
    pub fn take_own(&mut self) -> Vec<(Frame, RecipientTag)> {
        std::mem::take(&mut self.own_out)
    }

    /// Whether the tags to ask relays for changed since the last call.
    pub fn take_tags_changed(&mut self) -> bool {
        std::mem::take(&mut self.tags_changed)
    }

    /// Rebuilds the tags on next use, and tells the node to renew its
    /// subscription.
    fn forget_tags(&mut self) {
        self.tags_day = None;
        self.tags_changed = true;
    }

    /// Opens a bundle as an introduction to us (§5.5). The sender becomes a
    /// request unless they are a contact already. Returns `None` if it is not
    /// one.
    fn introduce(&mut self, bundle: &Bundle, now: u32) -> Result<Option<Handled>> {
        let Ok(intro) = open_introduction(&self.identity, bundle) else {
            return Ok(None);
        };
        self.store
            .insert_tombstone(&intro.tombstone, bundle.expires_at)?;
        let relay = vec![Frame::Tombstone {
            hop_limit: DEFAULT_HOP_LIMIT,
            tombstone: intro.tombstone,
        }];
        let sender = intro.card.dh_key();
        if sender == &self.identity.dh_public() || self.store.is_blocked(sender.as_bytes())? {
            return Ok(Some(Handled {
                outcome: Outcome::Dropped,
                relay,
                events: Vec::new(),
            }));
        }

        let existing = self
            .contacts
            .iter()
            .position(|contact| contact.card.dh_key() == sender);
        let index = match existing {
            Some(index) => index,
            None => {
                let id = self.store.insert_contact(&intro.card, now, true)?;
                let keys = PairKeys::derive(&self.identity, sender)?;
                let session = Session::new(&self.identity, sender, &keys);
                self.contacts.push(Contact {
                    id,
                    card: intro.card.clone(),
                    keys,
                    request: true,
                    introduced: false,
                    hidden: false,
                    session,
                });
                self.forget_tags();
                self.contacts.len() - 1
            }
        };
        self.mark_introduced(index)?;
        let contact_id = self.contacts[index].id;

        let mut events = Vec::new();
        if let Some(message) = self.store.insert_message(
            contact_id,
            false,
            bundle.id,
            &intro.text,
            intro.created_at,
            Some(now),
        )? {
            events.push(Event::MessageReceived { message });
        }
        if existing.is_none() {
            events.push(Event::ContactRequest {
                contact: self.contact_view(contact_id)?,
            });
        }
        let mut handled = Handled {
            outcome: Outcome::Received,
            relay,
            events,
        };
        for opened in self.open_carried_from(contact_id, now)? {
            handled.relay.extend(opened.relay);
            handled.events.extend(opened.events);
        }
        Ok(Some(handled))
    }

    /// Opens the carried bundles that turn out to be from this contact.
    fn open_carried_from(&mut self, contact_id: i64, now: u32) -> Result<Vec<Handled>> {
        let mut opened = Vec::new();
        for bundle in self.store.carried_bundles(now)? {
            let Some(index) = self.contact_for_tag(&bundle.tag, now) else {
                continue;
            };
            if self.contacts[index].id == contact_id {
                if let Some(handled) = self.deliver(index, &bundle, now)? {
                    self.store.delete_bundle(bundle.id)?;
                    opened.push(handled);
                }
            }
        }
        Ok(opened)
    }

    /// Notes that a contact has our card: a bundle from them arrived, or one
    /// of ours was delivered to them.
    fn mark_introduced(&mut self, index: usize) -> Result<()> {
        if !self.contacts[index].introduced {
            self.store.set_introduced(self.contacts[index].id)?;
            self.contacts[index].introduced = true;
        }
        Ok(())
    }

    fn handle_tombstone(
        &mut self,
        hop_limit: u8,
        tombstone: Tombstone,
        now: u32,
    ) -> Result<Handled> {
        if !tombstone.verify() {
            return Ok(Handled::quiet(Outcome::Dropped));
        }
        if self.store.has_tombstone(tombstone.id)? {
            return Ok(Handled::quiet(Outcome::Duplicate));
        }
        let held = self.store.bundle(tombstone.id)?;
        let delivered = self.store.mark_delivered(tombstone.id, now)?;
        if held.is_none() && delivered.is_none() && !self.store.has_message(tombstone.id)? {
            // Anyone can make a tombstone for an id they made up (§6).
            return Ok(Handled::quiet(Outcome::Dropped));
        }
        self.store.delete_bundle(tombstone.id)?;
        let keep_until = held.map_or(now + MAX_FUTURE_EXPIRY_SECS, |bundle| bundle.expires_at);
        self.store.insert_tombstone(&tombstone, keep_until)?;
        if let Some((contact_id, _)) = delivered {
            if let Ok(index) = self.contact_index(contact_id) {
                self.mark_introduced(index)?;
            }
        }

        let relay = match hop_limit.checked_sub(1) {
            Some(hop_limit) => vec![Frame::Tombstone {
                hop_limit,
                tombstone,
            }],
            None => Vec::new(),
        };
        Ok(match delivered {
            Some((contact_id, message_id)) => Handled {
                outcome: Outcome::Delivered,
                relay,
                events: vec![Event::MessageDelivered {
                    contact_id,
                    message_id,
                }],
            },
            None => Handled {
                outcome: Outcome::Tombstoned,
                relay,
                events: Vec::new(),
            },
        })
    }

    fn seen(&self, id: BundleId) -> Result<bool> {
        Ok(self.store.has_bundle(id)?
            || self.store.has_tombstone(id)?
            || self.store.has_message(id)?
            || self.store.has_group_message(id)?
            || self.store.has_post(id)?)
    }

    fn contact_for_tag(&mut self, tag: &RecipientTag, now: u32) -> Option<usize> {
        self.refresh_tags(now);
        self.tags.get(tag).copied()
    }

    /// Rebuilds the tags of bundles for us when the day or the contacts change.
    fn refresh_tags(&mut self, now: u32) {
        let today = day_of(now);
        if self.tags_day == Some(today) {
            return;
        }
        let me = self.identity.dh_public();
        self.tags.clear();
        for (index, contact) in self.contacts.iter().enumerate() {
            for day in tag_window(now) {
                self.tags
                    .insert(contact.keys.recipient_tag(&me, day), index);
            }
        }
        self.group_tags.clear();
        for (id, chain) in self.store.receiver_chains().unwrap_or_default() {
            for day in tag_window(now) {
                self.group_tags.insert(chain.tag(day), id);
            }
        }
        self.channel_tags.clear();
        for channel in self.store.channels().unwrap_or_default() {
            for day in tag_window(now) {
                self.channel_tags.insert(channel.keys.tag(day), channel.id);
            }
        }
        self.file_tags.clear();
        for (file, offer) in self.store.incomplete_files(now).unwrap_or_default() {
            let keys = offer.keys();
            for index in 0..offer.chunks() {
                self.file_tags.insert(keys.tag(index), (file, index));
            }
        }
        self.tags_day = Some(today);
    }

    /// A stored bundle as this node passes it on: in a chunk frame if it is a
    /// file chunk.
    fn passed_on(&self, bundle: &Bundle) -> Result<Option<Frame>> {
        Ok(if self.store.is_chunk(bundle.id)? {
            files::forwarded_chunk(bundle)
        } else {
            forwarded(bundle)
        })
    }

    fn contact_index(&self, contact_id: i64) -> Result<usize> {
        self.contacts
            .iter()
            .position(|contact| contact.id == contact_id)
            .ok_or(Error::Invalid("unknown contact"))
    }

    fn contact_view(&self, contact_id: i64) -> Result<ContactView> {
        let contact = &self.contacts[self.contact_index(contact_id)?];
        let last = self.store.last_message(contact_id)?;
        Ok(ContactView {
            id: contact.id,
            name: contact.card.name().to_owned(),
            fingerprint: contact.card.fingerprint().to_string(),
            last_message: last.as_ref().map(|(body, _)| body.clone()),
            last_at: last.map(|(_, at)| at),
            request: contact.request,
            unread: self.store.unread_count(contact_id)?,
            waiting: self.store.waiting_count(contact_id)?,
            secure: contact.session.is_established(),
            has_our_card: contact.introduced,
        })
    }
}

/// Contacts with their pair keys and sessions, as the store has them.
fn load_contacts(store: &Store, identity: &Identity) -> Result<Vec<Contact>> {
    let mut contacts = Vec::new();
    for stored in store.contacts()? {
        let keys = PairKeys::derive(identity, stored.card.dh_key())?;
        let session = match store.session(stored.id)? {
            Some(state) => Session::from_bytes(&state)?,
            None => Session::new(identity, stored.card.dh_key(), &keys),
        };
        contacts.push(Contact {
            id: stored.id,
            card: stored.card,
            keys,
            request: stored.request,
            introduced: stored.introduced,
            hidden: stored.hidden,
            session,
        });
    }
    Ok(contacts)
}

/// The days whose tags a receiver recognises (§4.1).
fn tag_window(now: u32) -> std::ops::RangeInclusive<u32> {
    let today = day_of(now);
    today.saturating_sub(TAG_WINDOW_PAST_DAYS)..=today + TAG_WINDOW_FUTURE_DAYS
}

/// The bundle as this node passes it on, with one hop used (§8).
fn forwarded(bundle: &Bundle) -> Option<Frame> {
    let hop_limit = bundle.hop_limit.checked_sub(1)?;
    Some(Frame::Bundle(Bundle {
        hop_limit,
        ..bundle.clone()
    }))
}

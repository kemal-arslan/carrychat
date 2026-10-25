//! What a node does with bundles and tombstones, independent of any link.
//! Every method takes the current time, so tests can control it.

use std::collections::HashMap;

use kervan_proto::bundle::{
    open, seal, Bundle, BundleId, Kind, Message, Tombstone, DEFAULT_HOP_LIMIT, DEFAULT_TTL_SECS,
    MAX_TTL_SECS,
};
use kervan_proto::frame::{Frame, Reassembler};
use kervan_proto::identity::{ContactCard, Identity};
use kervan_proto::intro::{mailbox_tag, open_introduction, seal_introduction};
use kervan_proto::pair::{PairKeys, RecipientTag, TAG_WINDOW_FUTURE_DAYS, TAG_WINDOW_PAST_DAYS};
use kervan_proto::{day_of, DAY_SECS};

use crate::store::Store;
use crate::{ContactView, Error, Event, MessageView, Profile, Result};

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
}

pub struct Engine {
    store: Store,
    identity: Identity,
    contacts: Vec<Contact>,
    /// Recipient tags of bundles for us, for the days in the tag window.
    tags: HashMap<RecipientTag, usize>,
    tags_day: Option<u32>,
    reassembler: Reassembler,
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
        let mut contacts = Vec::new();
        for stored in store.contacts()? {
            let keys = PairKeys::derive(&identity, stored.card.dh_key())?;
            contacts.push(Contact {
                id: stored.id,
                card: stored.card,
                keys,
                request: stored.request,
                introduced: stored.introduced,
            });
        }
        Ok(Self {
            store,
            identity,
            contacts,
            tags: HashMap::new(),
            tags_day: None,
            reassembler: Reassembler::default(),
        })
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
        } else {
            let keys = PairKeys::derive(&self.identity, card.dh_key())?;
            self.contacts.push(Contact {
                id,
                card,
                keys,
                request: false,
                introduced: false,
            });
        }
        self.tags_day = None;
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
        self.tags_day = None;
        Ok(())
    }

    pub fn contacts(&self) -> Result<Vec<ContactView>> {
        self.contacts
            .iter()
            .map(|contact| self.contact_view(contact.id))
            .collect()
    }

    pub fn messages(&self, contact_id: i64) -> Result<Vec<MessageView>> {
        self.contact_index(contact_id)?;
        self.store.messages(contact_id)
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
        let contact = &self.contacts[self.contact_index(contact_id)?];
        if contact.request {
            return Err(Error::Invalid("accept the request before you reply"));
        }
        let expires_at = now + DEFAULT_TTL_SECS;
        // Until the contact has our card, every message carries it (§5.5).
        let bundle = if contact.introduced {
            seal(
                &contact.keys,
                contact.card.dh_key(),
                &Message::text(text, now),
                expires_at,
                DEFAULT_HOP_LIMIT,
            )?
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
                    frames.extend(forwarded(&bundle));
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
        let mut frames: Vec<Frame> = self
            .store
            .forwardable_bundles(now)?
            .iter()
            .filter_map(forwarded)
            .collect();
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
            outbox.extend(forwarded(&bundle).map(|frame| (frame, tag)));
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
        self.store.bundle_counts()
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
        let Ok(message) = open(
            &self.contacts[index].keys,
            &self.identity.dh_public(),
            bundle,
        ) else {
            return Ok(None);
        };
        self.mark_introduced(index)?;
        let contact = &self.contacts[index];
        let tombstone = Tombstone::for_bundle(&contact.keys, bundle);
        self.store.insert_tombstone(&tombstone, bundle.expires_at)?;

        let mut events = Vec::new();
        if message.kind == Kind::Text {
            let body = String::from_utf8_lossy(&message.body);
            if let Some(view) = self.store.insert_message(
                contact.id,
                false,
                bundle.id,
                &body,
                message.created_at,
                Some(now),
            )? {
                events.push(Event::MessageReceived { message: view });
            }
        }
        Ok(Some(Handled {
            outcome: Outcome::Received,
            relay: vec![Frame::Tombstone {
                hop_limit: DEFAULT_HOP_LIMIT,
                tombstone,
            }],
            events,
        }))
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
                self.contacts.push(Contact {
                    id,
                    card: intro.card.clone(),
                    keys,
                    request: true,
                    introduced: false,
                });
                self.tags_day = None;
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
            || self.store.has_message(id)?)
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
        self.tags_day = Some(today);
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
        })
    }
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

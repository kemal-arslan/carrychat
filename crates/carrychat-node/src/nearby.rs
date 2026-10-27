//! Seeing people nearby (§9.6). A node that chose to be visible tells the
//! nodes it is directly connected to, over Bluetooth, Wi-Fi Aware or the
//! local network, who it is, but only to nodes that said they are visible
//! too. So two people see each other only if both turned it on, and a node
//! that did not never shows anyone its card. Nothing about it is passed on
//! to other nodes.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use carrychat_proto::identity::ContactCard;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::sync::Link;
use crate::{Event, Shared};

/// The sync frame type of nearby messages.
pub(crate) const NEARBY: u8 = 0x13;
/// "I am visible: who are you?"
const ASK: u8 = 0;
/// "This is me", followed by the contact card.
const ME: u8 = 1;
/// "I am no longer visible."
const GONE: u8 = 2;

/// Someone nearby who is visible, as this node is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NearbyPerson {
    /// Their node's session id in hex, which stays the same while they stay.
    pub session: String,
    pub name: String,
    pub fingerprint: String,
    /// Their `carrychat:` card, to add them as a contact.
    pub card: String,
    /// How they are connected.
    pub link: Link,
    /// Their contact id, if they are one of our contacts already.
    pub contact_id: Option<i64>,
}

pub(crate) struct Nearby {
    visible: AtomicBool,
    /// The visible nodes connected to this one, with their card and link.
    people: Mutex<HashMap<u64, (ContactCard, Link)>>,
    /// Nodes this one told its card to.
    told: Mutex<HashSet<u64>>,
}

impl Nearby {
    pub(crate) fn new(visible: bool) -> Self {
        Self {
            visible: AtomicBool::new(visible),
            people: Mutex::default(),
            told: Mutex::default(),
        }
    }

    pub(crate) fn visible(&self) -> bool {
        self.visible.load(Ordering::Relaxed)
    }

    /// The frame a visible node sends each node it connects to.
    pub(crate) fn ask() -> Vec<u8> {
        vec![NEARBY, ASK]
    }
}

impl Shared {
    /// Handles a nearby frame from `peer`. `outbox` reaches that peer.
    pub(crate) fn nearby_frame(
        &self,
        peer: u64,
        link: Link,
        outbox: &mpsc::UnboundedSender<Vec<u8>>,
        body: &[u8],
    ) {
        let nearby = &self.nearby;
        match body.split_first() {
            Some((&ASK, _)) if nearby.visible() => self.tell(peer, outbox),
            Some((&ME, card)) if nearby.visible() => {
                let Ok(card) = ContactCard::from_bytes(card) else {
                    return;
                };
                lock(&nearby.people).insert(peer, (card, link));
                // They answered our question, or asked theirs first.
                self.tell(peer, outbox);
                self.nearby_changed();
            }
            Some((&GONE, _)) => self.nearby_forget(peer),
            _ => {}
        }
    }

    /// Sends this node's card to `peer`, once.
    fn tell(&self, peer: u64, outbox: &mpsc::UnboundedSender<Vec<u8>>) {
        if !lock(&self.nearby.told).insert(peer) {
            return;
        }
        let mut frame = vec![NEARBY, ME];
        frame.extend_from_slice(&self.engine().card().to_bytes());
        let _ = outbox.send(frame);
    }

    /// A node went away, or stopped being visible.
    pub(crate) fn nearby_forget(&self, peer: u64) {
        lock(&self.nearby.told).remove(&peer);
        if lock(&self.nearby.people).remove(&peer).is_some() {
            self.nearby_changed();
        }
    }

    /// Becomes visible to visible nodes nearby, or stops.
    pub(crate) fn set_nearby_visible(&self, on: bool) {
        if self.nearby.visible.swap(on, Ordering::SeqCst) == on {
            return;
        }
        if on {
            self.peers.broadcast(&Nearby::ask(), None);
        } else {
            self.peers.broadcast(&[NEARBY, GONE], None);
            lock(&self.nearby.told).clear();
            lock(&self.nearby.people).clear();
            self.nearby_changed();
        }
    }

    pub(crate) fn nearby_people(&self) -> Vec<NearbyPerson> {
        let people: Vec<(u64, ContactCard, Link)> = lock(&self.nearby.people)
            .iter()
            .map(|(session, (card, link))| (*session, card.clone(), *link))
            .collect();
        let engine = self.engine();
        let mut people: Vec<NearbyPerson> = people
            .into_iter()
            .map(|(session, card, link)| NearbyPerson {
                session: format!("{session:016x}"),
                name: card.name().to_owned(),
                fingerprint: card.fingerprint().to_string(),
                card: card.to_text(),
                link,
                contact_id: engine.contact_id_of(&card),
            })
            .collect();
        people.sort_by(|a, b| a.name.cmp(&b.name));
        people
    }

    fn nearby_changed(&self) {
        let people = lock(&self.nearby.people).len();
        self.emit(Event::NearbyChanged { people });
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

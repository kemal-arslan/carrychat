//! Ratchet sessions between engines (§5.6), with frames passed by hand.

use carrychat_node::engine::{Engine, Handled, Outcome};
use carrychat_node::store::Store;
use carrychat_node::{DataKey, Event};
use carrychat_proto::frame::Frame;
use carrychat_proto::identity::ContactCard;
use tempfile::TempDir;

const NOW: u32 = 1_790_000_000;

/// An engine and its contact id for the other one.
struct Side {
    engine: Engine,
    peer: i64,
}

impl Side {
    fn send(&mut self, text: &str, at: u32) -> Frame {
        self.engine.send_text(self.peer, text, at).unwrap().1
    }

    fn secure(&self) -> bool {
        self.engine.contacts().unwrap()[0].secure
    }

    fn dh_key(&self) -> [u8; 32] {
        *ContactCard::from_text(&self.engine.profile().card)
            .unwrap()
            .dh_key()
            .as_bytes()
    }
}

/// Alice and Bob with each other's cards, after an introduction and its
/// receipt: from here on both seal pair bundles.
fn contacts(alice: Store, bob: Store) -> (Side, Side) {
    let mut alice = Engine::create(alice, "alice").unwrap();
    let mut bob = Engine::create(bob, "bob").unwrap();
    let bob_id = alice.add_contact(&bob.profile().card, NOW).unwrap().0.id;
    let alice_id = bob.add_contact(&alice.profile().card, NOW).unwrap().0.id;
    let (_, intro) = alice.send_text(bob_id, "selam", NOW).unwrap();
    let received = bob.handle(intro, NOW).unwrap();
    alice.handle(received.relay[0].clone(), NOW).unwrap();
    (
        Side {
            engine: alice,
            peer: bob_id,
        },
        Side {
            engine: bob,
            peer: alice_id,
        },
    )
}

fn initiator_and_responder() -> (Side, Side) {
    let (alice, bob) = contacts(
        Store::open_in_memory().unwrap(),
        Store::open_in_memory().unwrap(),
    );
    if alice.dh_key() < bob.dh_key() {
        (alice, bob)
    } else {
        (bob, alice)
    }
}

fn body(handled: &Handled) -> &str {
    handled
        .events
        .iter()
        .find_map(|event| match event {
            Event::MessageReceived { message } => Some(message.body.as_str()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a message, got {:?}", handled.events))
}

/// Whether the frame told the app that forward secrecy started.
fn turned_secure(handled: &Handled) -> bool {
    handled
        .events
        .iter()
        .any(|event| matches!(event, Event::ContactChanged { contact } if contact.secure))
}

fn len(frame: &Frame) -> usize {
    frame.to_bytes().len()
}

/// Sends a message and hands it over, with any session start that comes back.
fn talk(from: &mut Side, to: &mut Side, text: &str, at: u32) {
    let frame = from.send(text, at);
    let handled = to.engine.handle(frame, at).unwrap();
    assert_eq!(body(&handled), text);
    for (frame, _) in to.engine.take_own() {
        from.engine.handle(frame, at).unwrap();
    }
}

#[test]
fn the_initiator_starts_when_the_responder_writes_first() {
    let (mut initiator, mut responder) = initiator_and_responder();
    assert!(!initiator.secure() && !responder.secure());

    // No session yet, so the responder writes with the pair key.
    let first = responder.send("bir", NOW + 10);
    let handled = initiator.engine.handle(first.clone(), NOW + 10).unwrap();
    assert_eq!(body(&handled), "bir");
    let start = initiator.engine.take_own();
    assert_eq!(start.len(), 1);

    let started = responder
        .engine
        .handle(start[0].0.clone(), NOW + 11)
        .unwrap();
    assert_eq!(started.outcome, Outcome::Received);
    assert_eq!(started.events.len(), 1);
    assert!(turned_secure(&started));
    assert!(responder.secure());

    // From now on the responder writes in the session: 37 bytes of header more.
    let second = responder.send("iki", NOW + 12);
    assert_eq!(len(&second), len(&first) + 37);
    let handled = initiator.engine.handle(second, NOW + 12).unwrap();
    assert_eq!(body(&handled), "iki");
    assert!(turned_secure(&handled));
    assert!(initiator.secure());
    assert!(initiator.engine.take_own().is_empty());
}

#[test]
fn the_initiator_starts_with_its_own_message() {
    let (mut initiator, mut responder) = initiator_and_responder();
    let frame = initiator.send("merhaba", NOW + 10);
    assert_eq!(
        body(&responder.engine.handle(frame, NOW + 10).unwrap()),
        "merhaba"
    );
    assert!(responder.secure());
    assert!(!initiator.secure());
    assert!(responder.engine.take_own().is_empty());

    talk(&mut responder, &mut initiator, "hoş geldin", NOW + 11);
    assert!(initiator.secure());
}

#[test]
fn late_and_reordered_messages_open_once() {
    let (mut initiator, mut responder) = initiator_and_responder();
    let frames: Vec<Frame> = (0..4)
        .map(|n| initiator.send(&format!("mesaj {n}"), NOW + 10 + n))
        .collect();
    for n in [3, 0, 2] {
        let handled = responder
            .engine
            .handle(frames[n].clone(), NOW + 20)
            .unwrap();
        assert_eq!(body(&handled), format!("mesaj {n}"));
    }

    // Both ratchet forward; message 1 is still on its way.
    talk(&mut responder, &mut initiator, "tamam", NOW + 21);
    talk(&mut initiator, &mut responder, "yeni zincir", NOW + 22);

    let late = responder
        .engine
        .handle(frames[1].clone(), NOW + 30)
        .unwrap();
    assert_eq!(body(&late), "mesaj 1");
    assert_eq!(
        responder
            .engine
            .handle(frames[1].clone(), NOW + 31)
            .unwrap()
            .outcome,
        Outcome::Duplicate
    );
}

#[test]
fn sessions_survive_a_restart() {
    let dir = TempDir::new().unwrap();
    let paths = [dir.path().join("alice.db"), dir.path().join("bob.db")];
    let (mut alice, mut bob) = contacts(
        Store::open(&paths[0], &key()).unwrap(),
        Store::open(&paths[1], &key()).unwrap(),
    );
    talk(&mut alice, &mut bob, "bir", NOW + 10);
    talk(&mut bob, &mut alice, "iki", NOW + 11);
    talk(&mut alice, &mut bob, "üç", NOW + 12);
    assert!(alice.secure() && bob.secure());

    let reload = |side: Side, path| {
        let peer = side.peer;
        drop(side);
        Side {
            engine: Engine::load(Store::open(path, &key()).unwrap())
                .unwrap()
                .unwrap(),
            peer,
        }
    };
    let mut alice = reload(alice, &paths[0]);
    let mut bob = reload(bob, &paths[1]);
    assert!(alice.secure() && bob.secure());
    talk(&mut bob, &mut alice, "dört", NOW + 20);
    talk(&mut alice, &mut bob, "beş", NOW + 21);
}

/// The key these tests' databases are encrypted with, the same on every
/// opening.
fn key() -> DataKey {
    DataKey::from_bytes(&[7; 32]).unwrap()
}

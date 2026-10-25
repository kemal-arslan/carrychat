//! Delivery between engines, with frames passed by hand.

use kervan_node::engine::{Engine, Outcome};
use kervan_node::store::Store;
use kervan_node::Event;
use kervan_proto::bundle::{Bundle, BundleId, Tombstone, MAX_TTL_SECS};
use kervan_proto::frame::Frame;

const NOW: u32 = 1_790_000_000;

fn engine(name: &str) -> Engine {
    Engine::create(Store::open_in_memory().unwrap(), name).unwrap()
}

/// Alice and Bob, contacts of each other. Returns Bob's contact id at Alice's.
fn alice_and_bob() -> (Engine, Engine, i64) {
    let mut alice = engine("alice");
    let mut bob = engine("bob");
    let (bob_at_alice, _) = alice.add_contact(&bob.profile().card, NOW).unwrap();
    bob.add_contact(&alice.profile().card, NOW).unwrap();
    (alice, bob, bob_at_alice.id)
}

fn hop_limit(frame: &Frame) -> u8 {
    match frame {
        Frame::Bundle(bundle) | Frame::Chunk(bundle) => bundle.hop_limit,
        Frame::Tombstone { hop_limit, .. } => *hop_limit,
        Frame::Fragment(_) => panic!("unexpected fragment"),
    }
}

#[test]
fn direct_delivery_comes_back_as_a_receipt() {
    let (mut alice, mut bob, bob_id) = alice_and_bob();
    let (sent, frame) = alice.send_text(bob_id, "Merhaba, iyi misin?", NOW).unwrap();
    assert!(!sent.delivered);
    assert_eq!(alice.counts().unwrap(), (1, 0));

    let received = bob.handle(frame.clone(), NOW + 5).unwrap();
    assert_eq!(received.outcome, Outcome::Received);
    let [Event::MessageReceived { message }] = &received.events[..] else {
        panic!("expected one message, got {:?}", received.events);
    };
    assert_eq!(message.body, "Merhaba, iyi misin?");
    assert!(!message.outgoing);
    assert_eq!(
        bob.handle(frame, NOW + 6).unwrap().outcome,
        Outcome::Duplicate
    );

    let tombstone = received.relay.into_iter().next().unwrap();
    let delivered = alice.handle(tombstone, NOW + 10).unwrap();
    assert_eq!(delivered.outcome, Outcome::Delivered);
    assert_eq!(
        delivered.events,
        vec![Event::MessageDelivered {
            contact_id: bob_id,
            message_id: sent.id
        }]
    );
    assert!(alice.messages(bob_id).unwrap()[0].delivered);
    assert_eq!(alice.counts().unwrap(), (0, 0));
}

#[test]
fn a_stranger_carries_the_message_and_drops_it_after_delivery() {
    let (mut alice, mut bob, bob_id) = alice_and_bob();
    let mut carol = engine("carol");
    let (_, frame) = alice.send_text(bob_id, "Köprü yıkıldı", NOW).unwrap();

    let carried = carol.handle(frame.clone(), NOW).unwrap();
    assert_eq!(carried.outcome, Outcome::Carried);
    assert_eq!(hop_limit(&carried.relay[0]), hop_limit(&frame) - 1);
    assert_eq!(carol.counts().unwrap(), (0, 1));

    // An hour later Carol meets Bob and they sync.
    let later = NOW + 3600;
    let wanted = bob.missing(&carol.offer(later).unwrap()).unwrap();
    assert_eq!(wanted.len(), 1);
    let frames = carol.frames_for(&wanted, later).unwrap();
    let received = bob.handle(frames[0].clone(), later).unwrap();
    assert_eq!(received.outcome, Outcome::Received);

    let tombstone = received.relay[0].clone();
    let at_carol = carol.handle(tombstone, later).unwrap();
    assert_eq!(at_carol.outcome, Outcome::Tombstoned);
    assert_eq!(carol.counts().unwrap(), (0, 0));
    assert!(carol.offer(later).unwrap().is_empty());

    let delivered = alice.handle(at_carol.relay[0].clone(), later).unwrap();
    assert_eq!(delivered.outcome, Outcome::Delivered);
}

#[test]
fn only_the_sender_needs_a_card_and_the_recipient_sees_a_request() {
    let mut alice = engine("alice");
    let mut bob = engine("bob");
    let (bob_at_alice, _) = alice.add_contact(&bob.profile().card, NOW).unwrap();

    let (first, frame) = alice
        .send_text(bob_at_alice.id, "Ben Alice, kartımı aldın mı?", NOW)
        .unwrap();
    let received = bob.handle(frame, NOW).unwrap();
    assert_eq!(received.outcome, Outcome::Received);
    let [Event::MessageReceived { message }, Event::ContactRequest { contact }] =
        &received.events[..]
    else {
        panic!(
            "expected a message and a request, got {:?}",
            received.events
        );
    };
    assert_eq!(contact.name, "alice");
    assert!(contact.request);
    assert_eq!(message.contact_id, contact.id);
    assert_eq!(contact.fingerprint, alice.profile().fingerprint);
    let alice_id = contact.id;

    // The receipt tells Alice that Bob has her card now: she switches to pair bundles.
    let delivered = alice.handle(received.relay[0].clone(), NOW + 1).unwrap();
    assert_eq!(delivered.outcome, Outcome::Delivered);
    assert!(alice.messages(bob_at_alice.id).unwrap()[0].delivered);
    assert_eq!(first.body, "Ben Alice, kartımı aldın mı?");

    let (_, second) = alice
        .send_text(bob_at_alice.id, "Bir de bu", NOW + 2)
        .unwrap();
    let kept = bob.handle(second, NOW + 2).unwrap();
    assert_eq!(kept.outcome, Outcome::Received);
    assert!(kept
        .events
        .iter()
        .all(|event| !matches!(event, Event::ContactRequest { .. })));
    assert_eq!(bob.messages(alice_id).unwrap().len(), 2);

    // Bob cannot answer until he accepts.
    assert!(bob.send_text(alice_id, "Merhaba", NOW + 3).is_err());
    assert!(!bob.accept_contact(alice_id).unwrap().request);
    let (_, reply) = bob.send_text(alice_id, "Merhaba Alice", NOW + 3).unwrap();
    let at_alice = alice.handle(reply, NOW + 3).unwrap();
    assert_eq!(at_alice.outcome, Outcome::Received);
    assert_eq!(
        alice
            .messages(bob_at_alice.id)
            .unwrap()
            .last()
            .unwrap()
            .body,
        "Merhaba Alice"
    );
}

#[test]
fn a_declined_sender_is_ignored_until_added_by_hand() {
    let mut alice = engine("alice");
    let mut bob = engine("bob");
    let (bob_at_alice, _) = alice.add_contact(&bob.profile().card, NOW).unwrap();
    let (_, first) = alice.send_text(bob_at_alice.id, "Bir", NOW).unwrap();
    let (_, second) = alice.send_text(bob_at_alice.id, "İki", NOW).unwrap();

    let received = bob.handle(first, NOW).unwrap();
    let request = bob.contacts().unwrap()[0].id;
    bob.decline_contact(request).unwrap();
    assert!(bob.contacts().unwrap().is_empty());

    // Another introduction from the same sender is dropped, but still answered
    // with a tombstone so it stops travelling.
    let dropped = bob.handle(second, NOW + 1).unwrap();
    assert_eq!(dropped.outcome, Outcome::Dropped);
    assert!(dropped.events.is_empty());
    assert!(matches!(dropped.relay[..], [Frame::Tombstone { .. }]));

    // Alice learns Bob has her card and sends a pair bundle, which Bob cannot
    // place and carries. Adding her by hand opens it.
    alice.handle(received.relay[0].clone(), NOW + 2).unwrap();
    let (_, third) = alice.send_text(bob_at_alice.id, "Üç", NOW + 3).unwrap();
    assert_eq!(
        bob.handle(third, NOW + 3).unwrap().outcome,
        Outcome::Carried
    );
    let (alice_at_bob, opened) = bob.add_contact(&alice.profile().card, NOW + 4).unwrap();
    assert!(!alice_at_bob.request);
    assert_eq!(opened.len(), 1);
    assert_eq!(bob.messages(alice_at_bob.id).unwrap()[0].body, "Üç");
    assert_eq!(bob.counts().unwrap(), (0, 0));
}

#[test]
fn a_bundle_with_no_hops_left_is_kept_but_not_passed_on() {
    let (mut alice, _, bob_id) = alice_and_bob();
    let mut carol = engine("carol");
    let (_, frame) = alice.send_text(bob_id, "hi", NOW).unwrap();
    let Frame::Bundle(bundle) = frame else {
        panic!("expected a bundle");
    };
    let exhausted = Frame::Bundle(Bundle {
        hop_limit: 0,
        ..bundle
    });

    let carried = carol.handle(exhausted, NOW).unwrap();
    assert_eq!(carried.outcome, Outcome::Carried);
    assert!(carried.relay.is_empty());
    assert!(carol.offer(NOW).unwrap().is_empty());
    assert!(carol.export_frames(NOW).unwrap().is_empty());
}

#[test]
fn expired_and_far_future_bundles_are_dropped() {
    let (mut alice, _, bob_id) = alice_and_bob();
    let mut carol = engine("carol");
    let (_, frame) = alice.send_text(bob_id, "hi", NOW).unwrap();
    let Frame::Bundle(bundle) = &frame else {
        panic!("expected a bundle");
    };

    let expired = carol.handle(frame.clone(), bundle.expires_at).unwrap();
    assert_eq!(expired.outcome, Outcome::Dropped);
    let too_early = bundle.expires_at - MAX_TTL_SECS - 2 * 86_400;
    assert_eq!(
        carol.handle(frame, too_early).unwrap().outcome,
        Outcome::Dropped
    );
}

#[test]
fn tombstones_that_prove_nothing_are_dropped() {
    let (mut alice, _, bob_id) = alice_and_bob();
    let (_, frame) = alice.send_text(bob_id, "hi", NOW).unwrap();
    let Frame::Bundle(bundle) = frame else {
        panic!("expected a bundle");
    };

    let forged = Frame::Tombstone {
        hop_limit: 8,
        tombstone: Tombstone {
            id: bundle.id,
            ack_secret: [0x42; 16],
        },
    };
    assert_eq!(alice.handle(forged, NOW).unwrap().outcome, Outcome::Dropped);

    // Self-consistent, but for an id nobody has seen.
    let secret = [0x07; 16];
    let unknown = Frame::Tombstone {
        hop_limit: 8,
        tombstone: Tombstone {
            id: BundleId::from_ack_secret(&secret),
            ack_secret: secret,
        },
    };
    let handled = alice.handle(unknown, NOW).unwrap();
    assert_eq!(handled.outcome, Outcome::Dropped);
    assert!(handled.relay.is_empty());
    assert_eq!(alice.counts().unwrap(), (1, 0));
}

#[test]
fn own_card_and_garbage_are_refused_as_contacts() {
    let mut alice = engine("alice");
    let own = alice.profile().card;
    assert!(alice.add_contact(&own, NOW).is_err());
    assert!(alice.add_contact("kervan:not-a-card", NOW).is_err());
    assert!(alice.contacts().unwrap().is_empty());
}

#[test]
fn read_receipts_reach_the_sender_unless_turned_off() {
    let (mut alice, mut bob, bob_id) = alice_and_bob();
    let (sent, frame) = alice.send_text(bob_id, "Okudun mu?", NOW).unwrap();
    bob.handle(frame, NOW).unwrap();
    let alice_id = bob.contacts().unwrap()[0].id;
    assert_eq!(bob.contacts().unwrap()[0].unread, 1);

    let receipts = bob.mark_read(alice_id, NOW + 5).unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(bob.contacts().unwrap()[0].unread, 0);
    assert!(bob.messages(alice_id).unwrap()[0].read);
    assert!(bob.mark_read(alice_id, NOW + 6).unwrap().is_empty());

    // The receipt may also start forward secrecy at Alice's end (§5.6).
    let at_alice = alice.handle(receipts[0].0.clone(), NOW + 6).unwrap();
    assert!(at_alice.events.contains(&Event::MessagesRead {
        contact_id: bob_id,
        message_ids: vec![sent.id]
    }));
    assert!(alice.messages(bob_id).unwrap()[0].read);

    bob.set_read_receipts(false).unwrap();
    assert!(!bob.settings().unwrap().read_receipts);
    let (_, second) = alice.send_text(bob_id, "Ya bunu?", NOW + 10).unwrap();
    bob.handle(second, NOW + 10).unwrap();
    assert!(bob.mark_read(alice_id, NOW + 11).unwrap().is_empty());
    assert_eq!(bob.contacts().unwrap()[0].unread, 0);
}

#[test]
fn a_new_name_reaches_contacts_and_keeps_the_fingerprint() {
    let (mut alice, mut bob, bob_id) = alice_and_bob();
    let (_, frame) = alice.send_text(bob_id, "hi", NOW).unwrap();
    let received = bob.handle(frame, NOW).unwrap();
    alice.handle(received.relay[0].clone(), NOW).unwrap();

    let fingerprint = alice.profile().fingerprint;
    let (profile, updates) = alice.rename("Ayşe", NOW + 1).unwrap();
    assert_eq!(profile.name, "Ayşe");
    assert_eq!(profile.fingerprint, fingerprint);
    assert_eq!(updates.len(), 1);

    let changed = bob.handle(updates[0].0.clone(), NOW + 2).unwrap();
    let [Event::ContactChanged { contact }] = &changed.events[..] else {
        panic!("expected a changed contact, got {:?}", changed.events);
    };
    assert_eq!(contact.name, "Ayşe");
    assert_eq!(contact.fingerprint, fingerprint);
    assert!(alice.rename("   ", NOW + 3).is_err());
}

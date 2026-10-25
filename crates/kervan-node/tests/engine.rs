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
        Frame::Bundle(bundle) => bundle.hop_limit,
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
fn adding_a_contact_opens_bundles_already_carried_for_us() {
    let mut alice = engine("alice");
    let mut bob = engine("bob");
    let (bob_at_alice, _) = alice.add_contact(&bob.profile().card, NOW).unwrap();
    let (_, frame) = alice.send_text(bob_at_alice.id, "Su lazım", NOW).unwrap();

    assert_eq!(bob.handle(frame, NOW).unwrap().outcome, Outcome::Carried);
    let (alice_at_bob, opened) = bob.add_contact(&alice.profile().card, NOW + 60).unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].outcome, Outcome::Received);
    assert_eq!(bob.messages(alice_at_bob.id).unwrap()[0].body, "Su lazım");
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

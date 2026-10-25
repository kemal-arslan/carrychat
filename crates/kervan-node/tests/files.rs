//! Files between engines (§5.9): offers, chunks, carriers and receipts.

use std::collections::VecDeque;

use kervan_node::engine::{Engine, Outcome};
use kervan_node::store::Store;
use kervan_node::Event;
use kervan_proto::frame::Frame;

const NOW: u32 = 1_790_000_000;

fn engine(name: &str) -> Engine {
    Engine::create(Store::open_in_memory().unwrap(), name).unwrap()
}

fn photo(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 256) as u8).collect()
}

/// Alice and Bob, contacts who have each other's cards. Returns Bob's id at Alice's.
fn alice_and_bob() -> (Engine, Engine, i64) {
    let mut alice = engine("alice");
    let mut bob = engine("bob");
    let bob_id = alice.add_contact(&bob.profile().card, NOW).unwrap().0.id;
    bob.add_contact(&alice.profile().card, NOW).unwrap();
    let (_, intro) = alice.send_text(bob_id, "selam", NOW).unwrap();
    let received = bob.handle(intro, NOW).unwrap();
    alice.handle(received.relay[0].clone(), NOW).unwrap();
    (alice, bob, bob_id)
}

fn file_events(events: &[Event]) -> Vec<(u32, bool)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::FileChanged { file } => Some((file.received, file.ready)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_photo_reaches_a_contact_through_a_carrier_who_then_drops_it() {
    let (mut alice, mut bob, bob_id) = alice_and_bob();
    let mut carol = engine("carol");
    let data = photo(9_000);
    let (sent, frames) = alice
        .send_file(bob_id, "image/jpeg", &data, NOW + 10)
        .unwrap();
    let file = sent.file.clone().unwrap();
    assert_eq!((file.chunks, file.received, file.ready), (3, 3, true));
    assert_eq!(frames.len(), 4);
    assert!(matches!(frames[0].0, Frame::Bundle(_)));
    assert!(frames[1..]
        .iter()
        .all(|(frame, _)| matches!(frame, Frame::Chunk(_))));

    // Carol carries everything; chunks do not count among the messages she carries.
    for (frame, _) in &frames {
        assert_eq!(
            carol.handle(frame.clone(), NOW + 20).unwrap().outcome,
            Outcome::Carried
        );
    }
    assert_eq!(carol.counts().unwrap().1, 1);

    // Later Carol meets Bob: he asks for what he lacks, chunks come as chunk frames.
    let wanted = bob.missing(&carol.offer(NOW + 3600).unwrap()).unwrap();
    assert_eq!(wanted.len(), 4);
    let synced = carol.frames_for(&wanted, NOW + 3600).unwrap();
    assert_eq!(
        synced
            .iter()
            .filter(|frame| matches!(frame, Frame::Chunk(_)))
            .count(),
        3
    );
    let mut events = Vec::new();
    let mut tombstones = Vec::new();
    for frame in synced {
        let handled = bob.handle(frame, NOW + 3600).unwrap();
        events.extend(handled.events);
        tombstones.extend(handled.relay);
    }
    let [Event::MessageReceived { message }] = &events[..1] else {
        panic!("expected the offer first, got {events:?}");
    };
    let got = message.file.clone().unwrap();
    assert_eq!((got.mime.as_str(), got.size), ("image/jpeg", 9_000));
    assert_eq!(
        file_events(&events),
        vec![(1, false), (2, false), (3, true)]
    );
    let (mime, bytes) = bob.file_data(got.id).unwrap().unwrap();
    assert_eq!((mime.as_str(), bytes), ("image/jpeg", data));
    assert!(
        bob.messages(bob.contacts().unwrap()[0].id).unwrap()[1]
            .file
            .as_ref()
            .unwrap()
            .ready
    );

    // Bob's receipts let Carol drop all four, and tell Alice the offer arrived.
    assert_eq!(tombstones.len(), 4);
    for tombstone in &tombstones {
        carol.handle(tombstone.clone(), NOW + 3700).unwrap();
        alice.handle(tombstone.clone(), NOW + 3700).unwrap();
    }
    assert!(carol.offer(NOW + 3700).unwrap().is_empty());
    assert!(alice.messages(bob_id).unwrap()[1].delivered);
    assert!(alice.outbox(NOW + 3700).unwrap().is_empty());
}

#[test]
fn chunks_that_arrive_before_the_offer_are_kept_and_used() {
    let (mut alice, mut bob, bob_id) = alice_and_bob();
    let data = photo(4_001);
    let (_, frames) = alice
        .send_file(bob_id, "audio/mp4", &data, NOW + 10)
        .unwrap();
    for (frame, _) in &frames[1..] {
        assert_eq!(
            bob.handle(frame.clone(), NOW + 20).unwrap().outcome,
            Outcome::Carried
        );
    }
    let handled = bob.handle(frames[0].0.clone(), NOW + 30).unwrap();
    assert_eq!(file_events(&handled.events), vec![(1, false), (2, true)]);
    // The offer's receipt and both chunks'.
    assert_eq!(handled.relay.len(), 3);
    let file = bob.messages(bob.contacts().unwrap()[0].id).unwrap()[1]
        .file
        .clone()
        .unwrap();
    assert_eq!(bob.file_data(file.id).unwrap().unwrap().1, data);
    // The chunks he carried are gone. He may offer his own session start (§5.6).
    let offered = bob.offer(NOW + 40).unwrap();
    for (frame, _) in &frames[1..] {
        let Frame::Chunk(chunk) = frame else {
            panic!("expected a chunk");
        };
        assert!(!offered.contains(&chunk.id));
    }
}

#[test]
fn a_file_in_a_group_reaches_every_member_who_carries_it_on() {
    let mut nodes: Vec<Engine> = ["alice", "bob", "carol"]
        .iter()
        .map(|name| engine(name))
        .collect();
    let mut ids = Vec::new();
    for other in 1..3 {
        let card = nodes[other].profile().card;
        let id = nodes[0].add_contact(&card, NOW).unwrap().0.id;
        let alice_card = nodes[0].profile().card;
        nodes[other].add_contact(&alice_card, NOW).unwrap();
        let (_, intro) = nodes[0].send_text(id, "selam", NOW).unwrap();
        let received = nodes[other].handle(intro, NOW).unwrap();
        nodes[0].handle(received.relay[0].clone(), NOW).unwrap();
        ids.push(id);
    }
    let group = nodes[0].create_group("Mahalle", &ids, NOW).unwrap().id;
    flood(&mut nodes, 0, Vec::new());

    let data = photo(12_345);
    let (sent, frames) = nodes[0]
        .send_group_file(group, "image/jpeg", &data, NOW + 10)
        .unwrap();
    assert!(sent.file.unwrap().ready);
    let events = flood(
        &mut nodes,
        0,
        frames.into_iter().map(|(frame, _)| frame).collect(),
    );
    for member in 1..3 {
        let messages = nodes[member]
            .group_messages(nodes[member].groups().unwrap()[0].id)
            .unwrap();
        let file = messages.last().unwrap().file.clone().unwrap();
        assert!(file.ready, "member {member}");
        assert_eq!(nodes[member].file_data(file.id).unwrap().unwrap().1, data);
        // No receipts in a group: members keep the chunks to pass on.
        assert!(nodes[member].offer(NOW + 20).unwrap().len() >= 4);
        assert!(events[member]
            .iter()
            .any(|event| matches!(event, Event::FileChanged { file } if file.ready)));
    }
}

#[test]
fn a_file_posted_to_a_channel_reaches_who_joined_it() {
    let mut nodes = vec![engine("alice"), engine("bob")];
    let alice = nodes[0]
        .join_channel("mahalle", "", "Ayşe", NOW)
        .unwrap()
        .0
        .id;
    let bob = nodes[1]
        .join_channel("mahalle", "", "Bora", NOW)
        .unwrap()
        .0
        .id;
    let data = photo(5_000);
    let (_, frames) = nodes[0]
        .send_channel_file(alice, "image/jpeg", &data, NOW + 10)
        .unwrap();
    // A channel post's chunks start with the channel's hop limit.
    assert!(matches!(&frames[1].0, Frame::Chunk(chunk) if chunk.hop_limit == 3));
    flood(
        &mut nodes,
        0,
        frames.into_iter().map(|(frame, _)| frame).collect(),
    );
    let post = nodes[1].posts(bob).unwrap().pop().unwrap();
    let file = post.file.unwrap();
    assert!(file.ready);
    assert_eq!(nodes[1].file_data(file.id).unwrap().unwrap().1, data);
}

#[test]
fn files_must_be_one_mib_at_most_and_not_empty() {
    let (mut alice, _, bob_id) = alice_and_bob();
    assert!(alice.send_file(bob_id, "image/jpeg", &[], NOW).is_err());
    assert!(alice
        .send_file(bob_id, "image/jpeg", &photo((1 << 20) + 1), NOW)
        .is_err());
    let (sent, frames) = alice
        .send_file(bob_id, "image/jpeg", &photo(1 << 20), NOW)
        .unwrap();
    assert_eq!(sent.file.unwrap().chunks, 263);
    assert_eq!(frames.len(), 264);
    // An exported file carries chunks as chunk frames.
    let exported = alice.export_frames(NOW + 1).unwrap();
    assert_eq!(
        exported
            .iter()
            .filter(|frame| matches!(frame, Frame::Chunk(_)))
            .count(),
        263
    );
}

/// Passes frames from `from` to every other node until nothing new moves.
/// Returns what each node told its app.
fn flood(nodes: &mut [Engine], from: usize, frames: Vec<Frame>) -> Vec<Vec<Event>> {
    let mut events = vec![Vec::new(); nodes.len()];
    let mut queue: VecDeque<(usize, Frame)> = frames.into_iter().map(|f| (from, f)).collect();
    queue.extend(nodes[from].take_own().into_iter().map(|(f, _)| (from, f)));
    while let Some((sender, frame)) = queue.pop_front() {
        for node in (0..nodes.len()).filter(|&node| node != sender) {
            let handled = nodes[node].handle(frame.clone(), NOW + 60).unwrap();
            events[node].extend(handled.events);
            queue.extend(handled.relay.into_iter().map(|f| (node, f)));
            queue.extend(nodes[node].take_own().into_iter().map(|(f, _)| (node, f)));
        }
    }
    events
}

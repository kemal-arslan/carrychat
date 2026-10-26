//! Nodes on disk, talking over files and the local network.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use carrychat_node::{Event, LanConfig, Link, Node, Via};
use tempfile::TempDir;
use tokio::sync::broadcast;

struct TestNode {
    node: Node,
    _dir: TempDir,
}

fn node(name: &str) -> TestNode {
    let dir = TempDir::new().unwrap();
    TestNode {
        node: Node::create(dir.path(), name).unwrap(),
        _dir: dir,
    }
}

/// Makes `a` and `b` contacts of each other. Returns `b`'s id at `a` and `a`'s
/// id at `b`.
fn introduce(a: &Node, b: &Node) -> (i64, i64) {
    let b_at_a = a.add_contact(&b.profile().card).unwrap().id;
    let a_at_b = b.add_contact(&a.profile().card).unwrap().id;
    (b_at_a, a_at_b)
}

async fn next_event(events: &mut broadcast::Receiver<Event>, wanted: fn(&Event) -> bool) -> Event {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let event = events.recv().await.unwrap();
            if wanted(&event) {
                return event;
            }
        }
    })
    .await
    .expect("event did not arrive in time")
}

fn no_discovery() -> LanConfig {
    LanConfig {
        discovery: None,
        listen_port: 0,
    }
}

fn local(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

#[test]
fn a_usb_stick_carries_a_message_and_its_receipt() {
    let alice = node("alice");
    let bob = node("bob");
    let carol = node("carol");
    let (bob_id, _) = introduce(&alice.node, &bob.node);
    let stick = TempDir::new().unwrap();
    let file = stick.path().join("messages.carrychat");

    alice
        .node
        .send_text(bob_id, "Kadıköy'deyim, iyiyim.")
        .unwrap();
    assert_eq!(alice.node.export_file(&file).unwrap(), 1);
    assert_eq!(carol.node.import_file(&file).unwrap().carried, 1);

    carol.node.export_file(&file).unwrap();
    let at_bob = bob.node.import_file(&file).unwrap();
    assert_eq!(at_bob.received, 1);

    bob.node.export_file(&file).unwrap();
    assert_eq!(carol.node.import_file(&file).unwrap().carried, 0);
    carol.node.export_file(&file).unwrap();
    assert_eq!(alice.node.import_file(&file).unwrap().delivered, 1);
    assert!(alice.node.messages(bob_id).unwrap()[0].delivered);
    assert_eq!(carol.node.status().unwrap().carried, 0);
}

#[test]
fn a_file_that_is_not_carrychat_is_refused() {
    let alice = node("alice");
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("photo.jpg");
    std::fs::write(&file, b"\xff\xd8\xff\xe0 not a carrychat file").unwrap();
    assert!(alice.node.import_file(&file).is_err());
}

#[test]
fn a_node_reopens_with_its_identity_and_history() {
    let dir = TempDir::new().unwrap();
    let bob = node("bob");
    let card = {
        let alice = Node::create(dir.path(), "alice").unwrap();
        let bob_id = alice.add_contact(&bob.node.profile().card).unwrap().id;
        alice.send_text(bob_id, "hi").unwrap();
        alice.profile().card
    };
    let alice = Node::open(dir.path()).unwrap().expect("node exists");
    assert_eq!(alice.profile().card, card);
    let contacts = alice.contacts().unwrap();
    assert_eq!(contacts[0].last_message.as_deref(), Some("hi"));
    assert_eq!(alice.status().unwrap().outbox, 1);
    assert!(Node::create(dir.path(), "again").is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_messages_sync_when_peers_connect_and_new_ones_flow_live() {
    let alice = node("alice");
    let bob = node("bob");
    let (bob_id, alice_id) = introduce(&alice.node, &bob.node);
    let mut alice_events = alice.node.subscribe();
    let mut bob_events = bob.node.subscribe();

    let queued = alice.node.send_text(bob_id, "Ulaşırsa haber ver").unwrap();
    let alice_port = alice.node.start(no_discovery()).await.unwrap();
    bob.node.start(no_discovery()).await.unwrap();
    bob.node.connect(local(alice_port)).await.unwrap();

    let Event::MessageReceived { message } = next_event(&mut bob_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(message.body, "Ulaşırsa haber ver");
    assert_eq!(message.contact_id, alice_id);
    let delivered = next_event(&mut alice_events, |event| {
        matches!(event, Event::MessageDelivered { .. })
    })
    .await;
    assert_eq!(
        delivered,
        Event::MessageDelivered {
            contact_id: bob_id,
            message_id: queued.id
        }
    );

    bob.node.send_text(alice_id, "Ulaştı!").unwrap();
    let Event::MessageReceived { message } = next_event(&mut alice_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(message.body, "Ulaştı!");
    let status = alice.node.status().unwrap();
    assert_eq!(status.lan_peers, 1);
    assert_eq!(status.lan_problem, None);
}

/// A connection platform code opened, such as Bluetooth LE on Android, with
/// a small buffer so frames cross it in pieces as they do over the air.
fn platform_link(a: &Node, b: &Node) -> (tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>) {
    let (a_end, b_end) = tokio::io::duplex(64);
    let (a_reader, a_writer) = tokio::io::split(a_end);
    let (b_reader, b_writer) = tokio::io::split(b_end);
    let (a, b) = (a.clone(), b.clone());
    (
        tokio::spawn(async move { a.sync_over(Link::Bluetooth, a_reader, a_writer, None).await }),
        tokio::spawn(async move { b.sync_over(Link::Bluetooth, b_reader, b_writer, None).await }),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_platform_link_syncs_like_the_local_network() {
    let alice = node("alice");
    let bob = node("bob");
    let (bob_id, alice_id) = introduce(&alice.node, &bob.node);
    let mut alice_events = alice.node.subscribe();
    let mut bob_events = bob.node.subscribe();

    alice
        .node
        .send_text(bob_id, "Bluetooth'tan merhaba")
        .unwrap();
    let _link = platform_link(&alice.node, &bob.node);
    let Event::MessageReceived { message } = next_event(&mut bob_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(message.body, "Bluetooth'tan merhaba");

    bob.node.send_text(alice_id, "Aldım").unwrap();
    let Event::MessageReceived { message } = next_event(&mut alice_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(message.body, "Aldım");
    let status = alice.node.status().unwrap();
    assert_eq!((status.bluetooth_peers, status.lan_peers), (1, 0));

    // The two are connected already: a second connection, over any link, ends.
    let (second_a, second_b) = platform_link(&alice.node, &bob.node);
    tokio::time::timeout(Duration::from_secs(10), async {
        second_a.await.unwrap();
        second_b.await.unwrap();
    })
    .await
    .expect("the second connection should close");
    assert_eq!(alice.node.status().unwrap().bluetooth_peers, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_keeps_the_ways_it_left_by() {
    let alice = node("alice");
    let bob = node("bob");
    let carol = node("carol");
    let (bob_id, _) = introduce(&alice.node, &bob.node);
    let stick = TempDir::new().unwrap();
    let file = stick.path().join("messages.carrychat");

    let sent = alice.node.send_text(bob_id, "Yoldayım").unwrap();
    assert!(sent.via.is_empty());
    assert_eq!(alice.node.contacts().unwrap()[0].waiting, 1);

    alice.node.export_file(&file).unwrap();
    let mut alice_events = alice.node.subscribe();
    // Carol, nearby, takes it over Bluetooth to carry it on.
    let _link = platform_link(&alice.node, &carol.node);
    let moved = next_event(&mut alice_events, |event| {
        matches!(event, Event::MessageMoved { .. })
    })
    .await;
    assert_eq!(
        moved,
        Event::MessageMoved {
            contact_id: bob_id,
            message_id: sent.id
        }
    );
    assert_eq!(
        alice.node.messages(bob_id).unwrap()[0].via,
        vec![Via::File, Via::Bluetooth]
    );

    // The file reaches Bob; his receipt comes back on another file.
    assert_eq!(bob.node.import_file(&file).unwrap().received, 1);
    bob.node.export_file(&file).unwrap();
    assert_eq!(alice.node.import_file(&file).unwrap().delivered, 1);

    let journey = alice.node.journey(sent.id).unwrap();
    assert_eq!(journey.created_at, sent.created_at);
    assert!(journey.delivered_at.is_some());
    let ways: Vec<(Via, usize)> = journey
        .steps
        .iter()
        .map(|step| (step.via, step.count))
        .collect();
    assert_eq!(ways, vec![(Via::File, 1), (Via::Bluetooth, 1)]);
    assert_eq!(alice.node.contacts().unwrap()[0].waiting, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_in_the_middle_relays_between_two_others() {
    let alice = node("alice");
    let bob = node("bob");
    let carol = node("carol");
    let (bob_id, _) = introduce(&alice.node, &bob.node);
    let mut bob_events = bob.node.subscribe();
    let mut alice_events = alice.node.subscribe();

    let carol_port = carol.node.start(no_discovery()).await.unwrap();
    alice.node.start(no_discovery()).await.unwrap();
    bob.node.start(no_discovery()).await.unwrap();
    alice.node.connect(local(carol_port)).await.unwrap();
    bob.node.connect(local(carol_port)).await.unwrap();
    let mut carol_events = carol.node.subscribe();
    while carol.node.status().unwrap().lan_peers < 2 {
        next_event(&mut carol_events, |event| {
            matches!(event, Event::PeersChanged { .. })
        })
        .await;
    }

    alice.node.send_text(bob_id, "Carol üzerinden").unwrap();
    next_event(&mut bob_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await;
    next_event(&mut alice_events, |event| {
        matches!(event, Event::MessageDelivered { .. })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn nodes_find_each_other_by_multicast() {
    // A port of its own, so a running app does not join the test.
    let group = SocketAddrV4::new(
        Ipv4Addr::new(239, 255, 74, 74),
        47000 + std::process::id() as u16 % 1000,
    );
    let config = LanConfig {
        discovery: Some(group),
        listen_port: 0,
    };
    let alice = node("alice");
    let bob = node("bob");
    let mut alice_events = alice.node.subscribe();
    alice.node.start(config.clone()).await.unwrap();
    bob.node.start(config).await.unwrap();
    next_event(&mut alice_events, |event| {
        matches!(event, Event::PeersChanged { lan: 1, .. })
    })
    .await;
    assert_eq!(alice.node.status().unwrap().lan_problem, None);
}

#[test]
fn erasing_leaves_nothing_behind() {
    let dir = TempDir::new().unwrap();
    let card = Node::create(dir.path(), "alice").unwrap().profile().card;
    carrychat_node::erase(dir.path()).unwrap();
    assert!(Node::open(dir.path()).unwrap().is_none());
    let again = Node::create(dir.path(), "alice").unwrap();
    assert_ne!(again.profile().card, card);
}

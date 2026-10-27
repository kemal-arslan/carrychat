//! Nodes that reach each other only over Reticulum. A small transport node
//! inside the test plays the network, as RNS 1.5.5 behaves towards its TCP
//! clients: it passes each client's announces on to every client as its
//! own transport packets, does not replay announces to a client that joins
//! later, answers path requests from what it heard, forwards a data packet
//! only when it is named as the next hop, and hands it over with the first
//! header.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use carrychat_node::reticulum::wire::{self, Packet, PacketType};
use carrychat_node::{Event, Node};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};

const TRANSPORT_ID: [u8; wire::HASH_LEN] = [0xAA; wire::HASH_LEN];
const PATH_REQUEST: [u8; wire::HASH_LEN] = [
    0x6b, 0x9f, 0x66, 0x01, 0x4d, 0x98, 0x53, 0xfa, 0xab, 0x22, 0x0f, 0xba, 0x47, 0xd0, 0x27, 0x61,
];

#[derive(Default)]
struct Network {
    clients: HashMap<usize, mpsc::UnboundedSender<Vec<u8>>>,
    /// Each destination's client, and its announce as that client sent it.
    paths: HashMap<[u8; wire::HASH_LEN], (usize, Vec<u8>)>,
    /// Data packets dropped for not naming this node.
    dropped: usize,
}

/// An announce as this node passes it on: second header, its own transport
/// id, one hop.
fn passed_on(raw: &[u8], context: u8) -> Vec<u8> {
    let mut out = vec![0x40 | 0x10 | (raw[0] & 0x2F), 1];
    out.extend_from_slice(&TRANSPORT_ID);
    out.extend_from_slice(&raw[2..18]);
    out.push(context);
    out.extend_from_slice(&raw[19..]);
    out
}

fn received(network: &Mutex<Network>, client: usize, raw: Vec<u8>) {
    let mut network = network.lock().unwrap();
    let packet = Packet::parse(&raw).expect("clients send packets that parse");
    match packet.packet_type {
        PacketType::Announce => {
            network
                .paths
                .insert(packet.destination, (client, raw.clone()));
            let out = wire::frame(&passed_on(&raw, 0x00));
            for sender in network.clients.values() {
                let _ = sender.send(out.clone());
            }
        }
        PacketType::Data if packet.destination == PATH_REQUEST => {
            let target: [u8; wire::HASH_LEN] = packet.data[..16].try_into().unwrap();
            if let Some((_, announce)) = network.paths.get(&target) {
                let out = wire::frame(&passed_on(announce, 0x0B));
                let _ = network.clients[&client].send(out);
            }
        }
        PacketType::Data => {
            let path = network.paths.get(&packet.destination).map(|(to, _)| *to);
            match (packet.transport_id, path) {
                (Some(TRANSPORT_ID), Some(to)) => {
                    let mut out = vec![raw[0] & 0x0F, raw[1] + 1];
                    out.extend_from_slice(&raw[18..]);
                    let _ = network.clients[&to].send(wire::frame(&out));
                }
                _ => network.dropped += 1,
            }
        }
        _ => {}
    }
}

async fn start_network() -> (String, Arc<Mutex<Network>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let network = Arc::new(Mutex::new(Network::default()));
    let shared = network.clone();
    tokio::spawn(async move {
        let mut number = 0;
        while let Ok((stream, _)) = listener.accept().await {
            number += 1;
            let client = number;
            let (sender, mut outgoing) = mpsc::unbounded_channel::<Vec<u8>>();
            shared.lock().unwrap().clients.insert(client, sender);
            let network = shared.clone();
            tokio::spawn(async move {
                let (mut reader, mut writer) = stream.into_split();
                let mut deframer = wire::Deframer::default();
                let mut buffer = [0; 4096];
                loop {
                    tokio::select! {
                        read = reader.read(&mut buffer) => {
                            let Ok(len @ 1..) = read else { break };
                            deframer.push(&buffer[..len]);
                            while let Some(raw) = deframer.next_frame() {
                                received(&network, client, raw);
                            }
                        }
                        out = outgoing.recv() => {
                            let Some(out) = out else { break };
                            if writer.write_all(&out).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                let mut network = network.lock().unwrap();
                network.clients.remove(&client);
                network.paths.retain(|_, (owner, _)| *owner != client);
            });
        }
    });
    (address, network)
}

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

/// Waits until `node` has heard `count` other CarryChat nodes.
async fn hears(node: &Node, count: usize) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while node.reticulum().peers.len() < count {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the other node was not heard");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_nodes_talk_over_reticulum_only() {
    let (address, network) = start_network().await;
    let alice = node("alice");
    let bob = node("bob");
    let bob_id = alice.node.add_contact(&bob.node.profile().card).unwrap().id;
    let alice_id = bob.node.add_contact(&alice.node.profile().card).unwrap().id;
    alice.node.set_reticulum(Some(&address)).unwrap();
    hears(&alice.node, 0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Bob joins after Alice announced: the network does not tell him of her,
    // but she announces again on hearing him.
    bob.node.set_reticulum(Some(&address)).unwrap();
    hears(&alice.node, 1).await;
    hears(&bob.node, 1).await;
    let mut alice_events = alice.node.subscribe();
    let mut bob_events = bob.node.subscribe();

    let text = "Reticulum üzerinden merhaba.".repeat(20);
    alice.node.send_text(bob_id, &text).unwrap();
    let Event::MessageReceived { message } = next_event(&mut bob_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(message.body, text);
    assert_eq!(message.contact_id, alice_id);
    next_event(&mut alice_events, |event| {
        matches!(event, Event::MessageDelivered { .. })
    })
    .await;

    let status = alice.node.reticulum();
    assert!(status.connected);
    assert_eq!(status.peers[0].hops, 2);
    assert_eq!(
        status.peers[0].destination,
        bob.node.reticulum().destination.unwrap()
    );
    assert!(status.carrychat_at.is_some());
    assert_eq!(network.lock().unwrap().dropped, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restarted_node_asks_for_the_paths_it_knew() {
    let (address, _network) = start_network().await;
    let alice = node("alice");
    let bob = node("bob");
    let bob_id = alice.node.add_contact(&bob.node.profile().card).unwrap().id;
    bob.node.add_contact(&alice.node.profile().card).unwrap();
    bob.node.set_reticulum(Some(&address)).unwrap();
    alice.node.set_reticulum(Some(&address)).unwrap();
    hears(&alice.node, 1).await;
    hears(&bob.node, 1).await;

    // Alice stops and starts again: she remembers Bob, and the network
    // tells her where he is when she asks.
    alice.node.set_reticulum(None).unwrap();
    assert!(alice.node.reticulum().peers.is_empty());
    alice.node.set_reticulum(Some(&address)).unwrap();
    hears(&alice.node, 1).await;

    let mut bob_events = bob.node.subscribe();
    alice.node.send_text(bob_id, "Yeniden merhaba").unwrap();
    let Event::MessageReceived { message } = next_event(&mut bob_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(message.body, "Yeniden merhaba");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_written_before_the_link_started_goes_out() {
    let (address, _network) = start_network().await;
    let alice = node("alice");
    let bob = node("bob");
    let bob_id = alice.node.add_contact(&bob.node.profile().card).unwrap().id;
    bob.node.add_contact(&alice.node.profile().card).unwrap();
    alice
        .node
        .send_text(bob_id, "Reticulum açılmadan yazıldı")
        .unwrap();
    let mut bob_events = bob.node.subscribe();
    bob.node.set_reticulum(Some(&address)).unwrap();
    alice.node.set_reticulum(Some(&address)).unwrap();

    let Event::MessageReceived { message } = next_event(&mut bob_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(message.body, "Reticulum açılmadan yazıldı");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_third_node_learns_of_the_two_before_it() {
    let (address, _network) = start_network().await;
    let nodes = [node("alice"), node("bob"), node("carol")];
    for (index, test) in nodes.iter().enumerate() {
        test.node.set_reticulum(Some(&address)).unwrap();
        hears(&test.node, index).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    for test in &nodes {
        hears(&test.node, 2).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_back_after_a_short_absence_gets_what_it_missed() {
    let (address, network) = start_network().await;
    let alice = node("alice");
    let bob = node("bob");
    let bob_id = alice.node.add_contact(&bob.node.profile().card).unwrap().id;
    bob.node.add_contact(&alice.node.profile().card).unwrap();
    bob.node.set_reticulum(Some(&address)).unwrap();
    alice.node.set_reticulum(Some(&address)).unwrap();
    hears(&alice.node, 1).await;
    hears(&bob.node, 1).await;

    // Bob leaves for a moment; what Alice writes meanwhile finds no path.
    bob.node.set_reticulum(None).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    alice.node.send_text(bob_id, "Sen yokken yazıldı").unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        while network.lock().unwrap().dropped == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the message did not go out");

    // He is back long before Alice would count him as gone, and still gets
    // it.
    let mut bob_events = bob.node.subscribe();
    bob.node.set_reticulum(Some(&address)).unwrap();
    let Event::MessageReceived { message } = next_event(&mut bob_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(message.body, "Sen yokken yazıldı");
}

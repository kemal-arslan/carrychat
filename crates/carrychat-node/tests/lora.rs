//! Nodes that reach each other only over LoRa. A small radio inside the test
//! plays the Meshtastic devices: each TCP connection to it is one device,
//! and every packet a device sends, the others hear.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use carrychat_node::lora::meshtastic::{
    self, config, from_radio, mesh_packet, to_radio, Config, Deframer, FromRadio, LoRaConfig,
    MeshPacket, MyNodeInfo, NodeInfo, ToRadio, User,
};
use carrychat_node::{Event, LoraDevice, Node};
use prost::Message;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

/// Packets sent on the air, by the number of the device that sent them.
type Air = Arc<Mutex<Vec<(u32, Vec<u8>)>>>;

/// Starts the radio. Devices are numbered 1, 2, 3… in the order they connect.
async fn start_radio() -> (LoraDevice, Air) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let device = LoraDevice::Tcp(listener.local_addr().unwrap().to_string());
    let air: Air = Arc::default();
    let (waves, _) = broadcast::channel::<(u32, MeshPacket)>(256);
    let log = air.clone();
    tokio::spawn(async move {
        let mut number = 0;
        while let Ok((stream, _)) = listener.accept().await {
            number += 1;
            let me = number;
            let waves = waves.clone();
            let log = log.clone();
            tokio::spawn(async move {
                let (mut reader, mut writer) = stream.into_split();
                let mut heard = waves.subscribe();
                let mut deframer = Deframer::default();
                let mut buffer = [0; 1024];
                loop {
                    tokio::select! {
                        read = reader.read(&mut buffer) => {
                            let Ok(len @ 1..) = read else { return };
                            deframer.push(&buffer[..len]);
                            while let Some(body) = deframer.next_frame() {
                                let Ok(message) = ToRadio::decode(body.as_slice()) else { continue };
                                match message.payload {
                                    Some(to_radio::Payload::WantConfigId(id)) => {
                                        for reply in configuration(me, id) {
                                            writer.write_all(&meshtastic::frame(&reply)).await.unwrap();
                                        }
                                    }
                                    Some(to_radio::Payload::Packet(packet)) => {
                                        if let Some(mesh_packet::Payload::Decoded(data)) = &packet.payload {
                                            log.lock().unwrap().push((me, data.payload.clone()));
                                        }
                                        let _ = waves.send((me, packet));
                                    }
                                    _ => {}
                                }
                            }
                        }
                        wave = heard.recv() => {
                            let Ok((from, packet)) = wave else { return };
                            if from == me {
                                continue;
                            }
                            let packet = FromRadio {
                                id: 0,
                                payload: Some(from_radio::Payload::Packet(MeshPacket { from, ..packet })),
                            };
                            if writer.write_all(&meshtastic::frame(&packet)).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            });
        }
    });
    (device, air)
}

/// What a device answers `want_config_id`: its number, its own node with
/// its name, its radio settings, then the end of the configuration.
/// ShortFast keeps packets brief, so the airtime budget does not slow the
/// tests.
fn configuration(number: u32, id: u32) -> Vec<FromRadio> {
    let reply = |payload| FromRadio {
        id: 0,
        payload: Some(payload),
    };
    vec![
        reply(from_radio::Payload::MyInfo(MyNodeInfo {
            my_node_num: number,
        })),
        reply(from_radio::Payload::NodeInfo(NodeInfo {
            num: number,
            user: Some(User {
                long_name: format!("Device {number}"),
                ..User::default()
            }),
            ..NodeInfo::default()
        })),
        reply(from_radio::Payload::Config(Config {
            payload: Some(config::Payload::Lora(LoRaConfig {
                use_preset: true,
                modem_preset: 6,
                region: 3,
                hop_limit: 3,
                ..LoRaConfig::default()
            })),
        })),
        reply(from_radio::Payload::ConfigCompleteId(id)),
    ]
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

/// Connects a node to the radio and waits until its device is configured.
async fn tune_in(node: &Node, device: &LoraDevice) {
    let mut events = node.subscribe();
    node.start_lora(device.clone()).unwrap();
    next_event(&mut events, |event| {
        matches!(event, Event::LoraChanged { connected: true })
    })
    .await;
}

fn sent_by(air: &Air, device: u32) -> Vec<Vec<u8>> {
    air.lock()
        .unwrap()
        .iter()
        .filter(|(from, _)| *from == device)
        .map(|(_, payload)| payload.clone())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn two_nodes_talk_over_lora_only() {
    let (device, _air) = start_radio().await;
    let alice = node("alice");
    let bob = node("bob");
    let bob_id = alice.node.add_contact(&bob.node.profile().card).unwrap().id;
    let alice_id = bob.node.add_contact(&alice.node.profile().card).unwrap().id;
    tune_in(&alice.node, &device).await;
    tune_in(&bob.node, &device).await;
    let mut alice_events = alice.node.subscribe();
    let mut bob_events = bob.node.subscribe();

    alice.node.send_text(bob_id, "Telsizden merhaba").unwrap();
    let Event::MessageReceived { message } = next_event(&mut bob_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(message.body, "Telsizden merhaba");
    assert_eq!(message.contact_id, alice_id);
    // Bob's tombstone comes back over the radio as the receipt.
    next_event(&mut alice_events, |event| {
        matches!(event, Event::MessageDelivered { .. })
    })
    .await;
    let status = alice.node.status().unwrap();
    assert!(status.lora_connected);
    assert_eq!(status.lora_waiting, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_long_message_crosses_in_fragments() {
    let (device, air) = start_radio().await;
    let alice = node("alice");
    let bob = node("bob");
    let bob_id = alice.node.add_contact(&bob.node.profile().card).unwrap().id;
    bob.node.add_contact(&alice.node.profile().card).unwrap();
    tune_in(&alice.node, &device).await;
    tune_in(&bob.node, &device).await;
    let mut bob_events = bob.node.subscribe();

    let long = "Okulun bahçesinde çadır alanı kuruldu, su ve battaniye var. ".repeat(10);
    alice.node.send_text(bob_id, &long).unwrap();
    let Event::MessageReceived { message } = next_event(&mut bob_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(message.body, long.trim_end());
    let packets = sent_by(&air, 1);
    assert!(packets.len() >= 3, "{} packets", packets.len());
    assert!(packets.iter().all(|packet| packet.len() <= meshtastic::MTU));
}

#[tokio::test(flavor = "multi_thread")]
async fn what_comes_over_lora_is_not_sent_back_on_it() {
    let (device, air) = start_radio().await;
    let alice = node("alice");
    let bob = node("bob");
    let carol = node("carol");
    let bob_id = alice.node.add_contact(&bob.node.profile().card).unwrap().id;
    bob.node.add_contact(&alice.node.profile().card).unwrap();
    tune_in(&alice.node, &device).await;
    tune_in(&bob.node, &device).await;
    tune_in(&carol.node, &device).await;
    let mut alice_events = alice.node.subscribe();

    alice
        .node
        .send_text(bob_id, "Carol taşır ama yaymaz")
        .unwrap();
    next_event(&mut alice_events, |event| {
        matches!(event, Event::MessageDelivered { .. })
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    // Carol heard the bundle and the receipt, and keeps them for other
    // links, but the mesh has already flooded them: she sends nothing.
    assert!(sent_by(&air, 3).is_empty());
    assert!(!sent_by(&air, 2).is_empty(), "Bob sends his receipt");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_link_says_what_its_device_hears_and_can_be_turned_off() {
    let (device, _air) = start_radio().await;
    let alice = node("alice");
    let bob = node("bob");
    let bob_id = alice.node.add_contact(&bob.node.profile().card).unwrap().id;
    bob.node.add_contact(&alice.node.profile().card).unwrap();
    tune_in(&alice.node, &device).await;
    tune_in(&bob.node, &device).await;

    let status = alice.node.lora();
    assert!(status.connected);
    assert_eq!(status.device, Some(device.to_string()));
    assert_eq!(status.name.as_deref(), Some("Device 1"));
    assert_eq!(status.region.as_deref(), Some("EU_868"));
    assert!(status.heard.is_empty());
    assert_eq!(status.carrychat_at, None);

    // Alice's message is the first thing Bob's device hears from hers.
    let mut bob_events = bob.node.subscribe();
    alice.node.send_text(bob_id, "Telsizden").unwrap();
    next_event(&mut bob_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await;
    let status = bob.node.lora();
    let heard: Vec<&str> = status.heard.iter().map(|node| node.name.as_str()).collect();
    assert_eq!(heard, vec!["!00000001"]);
    assert!(status.carrychat_at.is_some());

    // Turned off, the link lets the device go and the choice is kept off.
    let mut alice_events = alice.node.subscribe();
    alice.node.set_lora(None).unwrap();
    next_event(&mut alice_events, |event| {
        matches!(event, Event::LoraChanged { connected: false })
    })
    .await;
    let status = alice.node.lora();
    assert_eq!(
        (status.device, status.connected, status.name),
        (None, false, None)
    );
    assert_eq!(alice.node.settings().unwrap().lora_device, None);

    // Chosen again, it connects and is remembered.
    alice.node.set_lora(Some(device.clone())).unwrap();
    next_event(&mut alice_events, |event| {
        matches!(event, Event::LoraChanged { connected: true })
    })
    .await;
    assert_eq!(
        alice.node.settings().unwrap().lora_device,
        Some(device.to_string())
    );
}

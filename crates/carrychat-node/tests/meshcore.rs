//! Nodes that reach each other only over LoRa through MeshCore devices. A
//! small radio inside the test plays them: each TCP connection to it is one
//! companion device, and every channel datagram a device floods, the
//! devices with the same channel hear.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use carrychat_node::lora::meshcore::{self, Deframer};
use carrychat_node::{Event, LoraDevice, LoraFirmware, Node};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

/// Datagrams flooded, as (device, channel secret, data type, data).
type Air = Arc<Mutex<Vec<(u8, [u8; 16], u16, Vec<u8>)>>>;

/// The public channel every MeshCore device starts with, in slot 0.
const PUBLIC: [u8; 16] = [
    0x8b, 0x33, 0x87, 0xe9, 0xc5, 0xcd, 0xea, 0x6a, 0xc9, 0xe5, 0xed, 0xba, 0xa1, 0x15, 0xcd, 0x72,
];

/// Starts the radio. Devices are numbered 1, 2, 3… in the order they
/// connect, and have `slots` channel slots, of which `taken` are in use.
async fn start_radio(slots: u8, taken: u8) -> (LoraDevice, Air) {
    start_radio_with(slots, taken, false).await
}

/// A radio whose devices, when `slow`, take long enough over their first
/// answer that the node asks again, and then answer both.
async fn start_radio_with(slots: u8, taken: u8, slow: bool) -> (LoraDevice, Air) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let device = LoraDevice::Tcp(listener.local_addr().unwrap().to_string());
    let air: Air = Arc::default();
    let (waves, _) = broadcast::channel::<(u8, [u8; 16], u16, Vec<u8>)>(256);
    let log = air.clone();
    tokio::spawn(async move {
        let mut number = 0;
        while let Ok((stream, _)) = listener.accept().await {
            number += 1;
            let mut device = Companion::new(number, slots, taken);
            // How many times a slow device has been asked what it is.
            let mut asked = if slow { 0 } else { u8::MAX };
            let waves = waves.clone();
            let log = log.clone();
            tokio::spawn(async move {
                let (mut reader, mut writer) = stream.into_split();
                let mut heard = waves.subscribe();
                let mut deframer = Deframer::new(meshcore::TO_DEVICE);
                let mut buffer = [0; 1024];
                loop {
                    let replies = tokio::select! {
                        read = reader.read(&mut buffer) => {
                            let Ok(len @ 1..) = read else { return };
                            deframer.push(&buffer[..len]);
                            let mut replies: Vec<Vec<u8>> = Vec::new();
                            while let Some(body) = deframer.next_frame() {
                                if let Some(sent) = device.flooded(&body) {
                                    log.lock().unwrap().push(sent.clone());
                                    let _ = waves.send(sent);
                                }
                                if asked < 2 && body[0] == meshcore::CMD_APP_START {
                                    asked += 1;
                                    // The first answer comes after the node asked
                                    // again, the second a moment after the first.
                                    let delay = if asked == 1 { 2500 } else { 300 };
                                    for reply in replies.drain(..) {
                                        let _ = writer.write_all(&meshcore::frame_from_device(&reply)).await;
                                    }
                                    tokio::time::sleep(Duration::from_millis(delay)).await;
                                }
                                replies.extend(device.answer(&body));
                            }
                            replies
                        }
                        wave = heard.recv() => {
                            let Ok(wave) = wave else { return };
                            device.hear(wave)
                        }
                    };
                    for reply in replies {
                        if writer
                            .write_all(&meshcore::frame_from_device(&reply))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            });
        }
    });
    (device, air)
}

/// One companion device: its channels, and the messages waiting on it.
struct Companion {
    number: u8,
    channels: Vec<(String, [u8; 16])>,
    waiting: VecDeque<Vec<u8>>,
}

impl Companion {
    fn new(number: u8, slots: u8, taken: u8) -> Self {
        let channels = (0..slots)
            .map(|slot| match slot {
                0 => ("Public".to_owned(), PUBLIC),
                slot if slot < taken => (format!("Group {slot}"), [slot; 16]),
                _ => (String::new(), [0; 16]),
            })
            .collect();
        Self {
            number,
            channels,
            waiting: VecDeque::new(),
        }
    }

    /// What the device sends back for a command.
    fn answer(&mut self, body: &[u8]) -> Vec<Vec<u8>> {
        match body[0] {
            meshcore::CMD_DEVICE_QUERY => {
                vec![vec![
                    meshcore::RESP_DEVICE_INFO,
                    9,
                    50,
                    self.channels.len() as u8,
                ]]
            }
            meshcore::CMD_APP_START => vec![self.self_info()],
            meshcore::CMD_SET_DEVICE_TIME => vec![vec![meshcore::RESP_OK]],
            meshcore::CMD_GET_CHANNEL => match self.channels.get(usize::from(body[1])) {
                Some((name, secret)) => {
                    let mut reply = vec![meshcore::RESP_CHANNEL_INFO, body[1]];
                    let mut padded = [0; 32];
                    padded[..name.len()].copy_from_slice(name.as_bytes());
                    reply.extend_from_slice(&padded);
                    reply.extend_from_slice(secret);
                    vec![reply]
                }
                None => vec![vec![meshcore::RESP_ERR, 2]],
            },
            meshcore::CMD_SET_CHANNEL => {
                let name = String::from_utf8_lossy(&body[2..34])
                    .trim_end_matches('\0')
                    .to_owned();
                let secret = body[34..50].try_into().unwrap();
                self.channels[usize::from(body[1])] = (name, secret);
                vec![vec![meshcore::RESP_OK]]
            }
            meshcore::CMD_GET_CONTACTS => vec![
                vec![meshcore::RESP_CONTACTS_START, 1, 0, 0, 0],
                contact("Tepe", 1),
                vec![meshcore::RESP_END_OF_CONTACTS, 0, 0, 0, 0],
            ],
            meshcore::CMD_SYNC_NEXT_MESSAGE => vec![self
                .waiting
                .pop_front()
                .unwrap_or(vec![meshcore::RESP_NO_MORE_MESSAGES])],
            meshcore::CMD_SEND_CHANNEL_DATA => vec![vec![meshcore::RESP_OK]],
            _ => vec![vec![meshcore::RESP_ERR, 1]],
        }
    }

    /// The datagram a command floods, if it is one.
    fn flooded(&self, body: &[u8]) -> Option<(u8, [u8; 16], u16, Vec<u8>)> {
        if body[0] != meshcore::CMD_SEND_CHANNEL_DATA {
            return None;
        }
        assert_eq!(body[2], meshcore::FLOOD);
        assert!(body.len() - 5 <= meshcore::MTU, "{} bytes", body.len() - 5);
        let (_, secret) = &self.channels[usize::from(body[1])];
        let data_type = u16::from_le_bytes([body[3], body[4]]);
        Some((self.number, *secret, data_type, body[5..].to_vec()))
    }

    /// A datagram on the air: kept if the device has its channel, and the
    /// app told that messages wait.
    fn hear(
        &mut self,
        (from, secret, data_type, data): (u8, [u8; 16], u16, Vec<u8>),
    ) -> Vec<Vec<u8>> {
        if from == self.number {
            return Vec::new();
        }
        let Some(slot) = self.channels.iter().position(|(_, held)| *held == secret) else {
            return Vec::new();
        };
        let mut message = vec![meshcore::RESP_CHANNEL_DATA, 40, 0, 0, slot as u8, 0];
        message.extend_from_slice(&data_type.to_le_bytes());
        message.push(data.len() as u8);
        message.extend_from_slice(&data);
        self.waiting.push_back(message);
        vec![vec![meshcore::PUSH_MSG_WAITING]]
    }

    /// What it says about itself. SF7 keeps packets brief, so the airtime
    /// budget does not slow the tests.
    fn self_info(&self) -> Vec<u8> {
        let mut reply = vec![meshcore::RESP_SELF_INFO, 1, 22, 22];
        reply.extend_from_slice(&[self.number; 32]);
        reply.extend_from_slice(&[0; 12]);
        reply.extend_from_slice(&869_525u32.to_le_bytes());
        reply.extend_from_slice(&250_000u32.to_le_bytes());
        reply.extend_from_slice(&[7, 5]);
        reply.extend_from_slice(format!("Companion {}", self.number).as_bytes());
        reply
    }
}

/// A repeater the device heard ten minutes ago, `hops` away.
fn contact(name: &str, hops: u8) -> Vec<u8> {
    let mut reply = vec![meshcore::RESP_CONTACT];
    reply.extend_from_slice(&[0xEE; 32]);
    reply.extend_from_slice(&[2, 0, hops]);
    reply.extend_from_slice(&[0; 64]);
    let mut padded = [0; 32];
    padded[..name.len()].copy_from_slice(name.as_bytes());
    reply.extend_from_slice(&padded);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32;
    let heard_at = now - 600;
    reply.extend_from_slice(&heard_at.to_le_bytes());
    reply.extend_from_slice(&[0; 8]);
    reply.extend_from_slice(&heard_at.to_le_bytes());
    reply
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

/// Connects a node to the radio and waits until its device is set up.
async fn tune_in(node: &Node, device: &LoraDevice) {
    let mut events = node.subscribe();
    node.start_lora(device.clone()).unwrap();
    next_event(&mut events, |event| {
        matches!(event, Event::LoraChanged { connected: true })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_nodes_talk_over_meshcore_only() {
    let (device, air) = start_radio(8, 1).await;
    let alice = node("alice");
    let bob = node("bob");
    let bob_id = alice.node.add_contact(&bob.node.profile().card).unwrap().id;
    let alice_id = bob.node.add_contact(&alice.node.profile().card).unwrap().id;
    tune_in(&alice.node, &device).await;
    tune_in(&bob.node, &device).await;
    let mut alice_events = alice.node.subscribe();
    let mut bob_events = bob.node.subscribe();

    // Long enough to need fragments of MeshCore's size.
    let text = "MeshCore üzerinden merhaba.".repeat(12);
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

    // Everything went on #carrychat, as CarryChat's datagrams.
    let air = air.lock().unwrap();
    assert!(air.iter().filter(|(from, ..)| *from == 1).count() > 1);
    for (_, secret, data_type, _) in air.iter() {
        assert_eq!(*secret, meshcore::channel_secret());
        assert_eq!(*data_type, meshcore::DATA_TYPE);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_link_says_what_a_meshcore_device_hears() {
    let (device, _air) = start_radio(8, 3).await;
    let alice = node("alice");
    tune_in(&alice.node, &device).await;

    let status = alice.node.lora();
    assert!(status.connected);
    assert_eq!(status.firmware, Some(LoraFirmware::MeshCore));
    assert_eq!(status.name.as_deref(), Some("Companion 1"));
    assert_eq!(status.radio.as_deref(), Some("869.525 MHz, SF7, 250 kHz"));
    assert_eq!(status.region, None);
    assert!(!status.no_free_channel);
    assert_eq!(status.heard_within, 48 * 60 * 60);
    let heard: Vec<_> = status
        .heard
        .iter()
        .map(|node| (node.name.as_str(), node.hops))
        .collect();
    assert_eq!(heard, [("Tepe", Some(1))]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_with_no_free_channel_slot_is_left_as_it_is() {
    let (device, air) = start_radio(4, 4).await;
    let alice = node("alice");
    let bob = node("bob");
    let bob_id = alice.node.add_contact(&bob.node.profile().card).unwrap().id;
    tune_in(&alice.node, &device).await;

    assert!(alice.node.lora().no_free_channel);
    alice.node.send_text(bob_id, "Kanal yok").unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(air.lock().unwrap().is_empty());
    assert!(alice.node.lora().waiting > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn late_answers_do_not_get_mixed_up() {
    // The device answers what it is late, so the node asks twice and both
    // answers come; the second must not pass for the answer to what the
    // node asks next.
    let (device, _air) = start_radio_with(8, 1, true).await;
    let alice = node("alice");
    tune_in(&alice.node, &device).await;
    let status = alice.node.lora();
    assert_eq!(status.firmware, Some(LoraFirmware::MeshCore));
    assert_eq!(status.radio.as_deref(), Some("869.525 MHz, SF7, 250 kHz"));
    assert!(!status.no_free_channel);
}

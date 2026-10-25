//! Two nodes that reach each other only through a Nostr relay, played by a
//! small relay inside the test, so nothing leaves the computer.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use kervan_node::{Event, InternetConfig, Node};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

type Events = Arc<Mutex<Vec<Value>>>;

/// A relay that stores events and serves REQ subscriptions with `kinds`,
/// `since` and tag filters, as NIP-01 describes.
async fn start_relay() -> (String, Events) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let stored: Events = Arc::default();
    let (fresh, _) = broadcast::channel::<Value>(256);
    let events = stored.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let stored = stored.clone();
            let fresh = fresh.clone();
            tokio::spawn(async move {
                let Ok(socket) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                let (mut sink, mut source) = socket.split();
                let mut subscriptions: Vec<(String, Vec<Value>)> = Vec::new();
                let mut new_events = fresh.subscribe();
                loop {
                    let replies = tokio::select! {
                        message = source.next() => {
                            let text = match message {
                                Some(Ok(Message::Text(text))) => text,
                                Some(Ok(_)) => continue,
                                _ => return,
                            };
                            let parts: Vec<Value> = serde_json::from_str(&text).unwrap();
                            match parts[0].as_str() {
                                Some("EVENT") => {
                                    let event = parts[1].clone();
                                    let is_new = {
                                        let mut stored = stored.lock().unwrap();
                                        let is_new = !stored.iter().any(|e| e["id"] == event["id"]);
                                        if is_new {
                                            stored.push(event.clone());
                                        }
                                        is_new
                                    };
                                    if is_new {
                                        let _ = fresh.send(event.clone());
                                    }
                                    vec![json!(["OK", event["id"], true, ""])]
                                }
                                Some("REQ") => {
                                    let id = parts[1].as_str().unwrap().to_owned();
                                    let filters = parts[2..].to_vec();
                                    let mut replies: Vec<Value> = stored
                                        .lock()
                                        .unwrap()
                                        .iter()
                                        .filter(|event| matches(event, &filters))
                                        .map(|event| json!(["EVENT", id, event]))
                                        .collect();
                                    replies.push(json!(["EOSE", id]));
                                    subscriptions.retain(|(existing, _)| *existing != id);
                                    subscriptions.push((id, filters));
                                    replies
                                }
                                _ => Vec::new(),
                            }
                        }
                        event = new_events.recv() => match event {
                            Ok(event) => subscriptions
                                .iter()
                                .filter(|(_, filters)| matches(&event, filters))
                                .map(|(id, _)| json!(["EVENT", id, event]))
                                .collect(),
                            Err(_) => Vec::new(),
                        },
                    };
                    for reply in replies {
                        if sink.send(Message::text(reply.to_string())).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    (url, events)
}

fn matches(event: &Value, filters: &[Value]) -> bool {
    filters.iter().any(|filter| {
        let kind_ok = filter["kinds"]
            .as_array()
            .is_none_or(|kinds| kinds.contains(&event["kind"]));
        let since_ok = filter["since"]
            .as_u64()
            .is_none_or(|since| event["created_at"].as_u64().unwrap() >= since);
        let tags_ok = filter.as_object().unwrap().iter().all(|(key, wanted)| {
            let Some(name) = key.strip_prefix('#') else {
                return true;
            };
            event["tags"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tag| tag[0] == name && wanted.as_array().unwrap().contains(&tag[1]))
        });
        kind_ok && since_ok && tags_ok
    })
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

#[tokio::test(flavor = "multi_thread")]
async fn two_nodes_meet_through_a_relay_only() {
    let (url, relay) = start_relay().await;
    let config = InternetConfig { relays: vec![url] };
    let alice = node("alice");
    let bob = node("bob");
    // Only Alice has Bob's card: her first message introduces her.
    let bob_id = alice.node.add_contact(&bob.node.profile().card).unwrap().id;
    let mut alice_events = alice.node.subscribe();
    let mut bob_events = bob.node.subscribe();

    // Alice writes before either node is online; it goes out on connecting.
    alice.node.send_text(bob_id, "İnternetten merhaba").unwrap();
    alice.node.start_internet(config.clone()).unwrap();
    bob.node.start_internet(config).unwrap();

    let Event::ContactRequest { contact } = next_event(&mut bob_events, |event| {
        matches!(event, Event::ContactRequest { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(contact.name, "alice");
    assert_eq!(
        bob.node.messages(contact.id).unwrap()[0].body,
        "İnternetten merhaba"
    );
    next_event(&mut alice_events, |event| {
        matches!(event, Event::MessageDelivered { .. })
    })
    .await;

    bob.node.accept_contact(contact.id).unwrap();
    bob.node
        .send_text(contact.id, "Aldım, teşekkürler")
        .unwrap();
    let Event::MessageReceived { message } = next_event(&mut alice_events, |event| {
        matches!(event, Event::MessageReceived { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(message.body, "Aldım, teşekkürler");
    next_event(&mut bob_events, |event| {
        matches!(event, Event::MessageDelivered { .. })
    })
    .await;

    let status = alice.node.status().unwrap();
    assert_eq!(status.internet_relays, 1);
    assert_eq!(status.outbox, 0);
    assert_eq!(status.internet_refusal, None);
    // Two bundles and two tombstones went through the relay, nothing in clear.
    let stored = relay.lock().unwrap();
    assert!(stored.len() >= 4);
    assert!(stored
        .iter()
        .all(|event| !event["content"].as_str().unwrap().contains("merhaba")));
}

#[tokio::test(flavor = "multi_thread")]
async fn junk_from_a_relay_is_not_carried() {
    let (url, _relay) = start_relay().await;
    let alice = node("alice");
    let bob = node("bob");
    let carol = node("carol");
    let bob_id = alice.node.add_contact(&bob.node.profile().card).unwrap().id;
    alice
        .node
        .start_internet(InternetConfig {
            relays: vec![url.clone()],
        })
        .unwrap();
    // Carol is subscribed to nothing Alice posts, and must not keep it either.
    carol
        .node
        .start_internet(InternetConfig { relays: vec![url] })
        .unwrap();
    alice.node.send_text(bob_id, "Bob için").unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(carol.node.status().unwrap().carried, 0);
    assert!(carol.node.contacts().unwrap().is_empty());
}

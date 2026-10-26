//! The internet link (§9.3): bundles and tombstones travel as Nostr events
//! through public relays. Every relay gets its own connection, which
//! reconnects on its own; what one connection receives is handled like a
//! frame from any other link.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use arti_client::config::TorClientConfigBuilder;
use arti_client::TorClient;
use serde::Serialize;

use carrychat_proto::bundle::{BundleId, MAX_TTL_SECS};
use carrychat_proto::frame::Frame;
use carrychat_proto::pair::RecipientTag;
use carrychat_proto::DAY_SECS;
use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use k256::schnorr::SigningKey;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, watch};
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::tungstenite::Message;

use crate::engine::{Handled, Outcome};
use crate::journey::Via;
use crate::{now, Error, Event, Result, Shared};

/// The Nostr event kind of CarryChat frames. Provisional, not registered.
pub const KIND: u32 = 9378;
pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.primal.net",
];

const SUBSCRIPTION_ID: &str = "carrychat";
/// How far back a subscription asks for events: the longest bundle lifetime
/// and a day.
const LOOKBACK_SECS: u32 = MAX_TTL_SECS + DAY_SECS;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RENEW_INTERVAL: Duration = Duration::from_secs(600);
const FIRST_RETRY: Duration = Duration::from_secs(2);
const MAX_RETRY: Duration = Duration::from_secs(60);
/// How long a relay has to answer after a message is sent before the
/// connection is taken as dead.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Time between two file chunks sent to one relay, so a photo does not run
/// into the relay's rate limit.
const CHUNK_GAP: Duration = Duration::from_millis(60);
/// Over Tor, circuits take longer to build and answers to come.
const TOR_CONNECT_TIMEOUT: Duration = Duration::from_secs(45);
const TOR_PROBE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long to wait before trying Tor again after it failed to start.
const TOR_RETRY: Duration = Duration::from_secs(30);

type Tor = TorClient<tor_rtcompat::PreferredRuntime>;

#[derive(Clone, Debug)]
pub struct InternetConfig {
    pub relays: Vec<String>,
    /// How often a relay is pinged. A relay silent for twice as long is taken
    /// as gone, as happens when the computer moves to another network, and is
    /// connected to again.
    pub keepalive: Duration,
    /// Reaches every relay over Tor, keeping Tor's state in this folder.
    /// Until Tor works the node does not connect at all, never directly.
    pub tor: Option<PathBuf>,
}

/// How the internet link stands with Tor, when it goes over Tor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum TorState {
    Starting,
    Ready,
    #[serde(rename_all = "camelCase")]
    Failed {
        detail: String,
    },
}

impl Default for InternetConfig {
    fn default() -> Self {
        Self {
            relays: DEFAULT_RELAYS
                .iter()
                .map(|relay| relay.to_string())
                .collect(),
            keepalive: Duration::from_secs(20),
            tor: None,
        }
    }
}

pub(crate) struct Internet {
    /// Messages for every connected relay.
    outgoing: broadcast::Sender<String>,
    /// File chunks to post, which each connection sends at a measured pace.
    chunks: broadcast::Sender<String>,
    /// Bumped when the subscription should be renewed.
    renew: watch::Sender<u64>,
    /// Bumped to make every connection prove it is alive.
    probe: watch::Sender<u64>,
    connected: AtomicUsize,
    secret: OnceLock<[u8; 32]>,
    /// The last reason a relay gave for refusing an event.
    refusal: Mutex<Option<String>>,
    /// Tor's state, when the link goes over Tor.
    tor_state: Mutex<Option<TorState>>,
    /// The Tor client, once it has started.
    tor: watch::Sender<Option<Arc<Tor>>>,
    /// The bundle each posted event carries, by event id, so a relay's OK
    /// tells which bundle it took.
    posted: Mutex<HashMap<String, BundleId>>,
}

/// Events remembered for their OK at most; past that the memory starts over.
const MAX_POSTED: usize = 4096;

impl Internet {
    pub(crate) fn new() -> Self {
        Self {
            outgoing: broadcast::channel(1024).0,
            chunks: broadcast::channel(8192).0,
            renew: watch::channel(0).0,
            probe: watch::channel(0).0,
            connected: AtomicUsize::new(0),
            secret: OnceLock::new(),
            refusal: Mutex::default(),
            tor_state: Mutex::default(),
            tor: watch::channel(None).0,
            posted: Mutex::default(),
        }
    }

    /// The `EVENT` message for a bundle or chunk frame, remembering which
    /// bundle the event carries.
    fn event_for(&self, secret: &[u8; 32], frame: &Frame, tag: &RecipientTag) -> Option<String> {
        let (id, message) = bundle_event(secret, frame, tag)?;
        if let Frame::Bundle(bundle) = frame {
            let mut posted = self
                .posted
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if posted.len() >= MAX_POSTED {
                posted.clear();
            }
            posted.insert(id, bundle.id);
        }
        Some(message)
    }

    pub(crate) fn tor_state(&self) -> Option<TorState> {
        self.tor_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn connected(&self) -> usize {
        self.connected.load(Ordering::Relaxed)
    }

    pub(crate) fn refusal(&self) -> Option<String> {
        self.refusal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn renew_subscription(&self) {
        self.renew.send_modify(|count| *count += 1);
    }

    /// Pings every relay and drops connections that do not answer within a
    /// few seconds, so a message sent into a dead connection, for example
    /// right after moving to another network, goes out again soon.
    pub(crate) fn probe(&self) {
        self.probe.send_modify(|count| *count += 1);
    }

    /// Posts a bundle or chunk frame under `tag` to every connected relay.
    pub(crate) fn post_bundle(&self, frame: &Frame, tag: &RecipientTag) {
        let Some(secret) = self.secret.get() else {
            return;
        };
        let Some(event) = self.event_for(secret, frame, tag) else {
            return;
        };
        let queue = match frame {
            Frame::Chunk(_) => &self.chunks,
            _ => &self.outgoing,
        };
        let _ = queue.send(event);
    }

    pub(crate) fn post_tombstone(&self, frame: &Frame) {
        let (Some(secret), Frame::Tombstone { tombstone, .. }) = (self.secret.get(), frame) else {
            return;
        };
        let event = signed_event(
            secret,
            now(),
            vec![vec!["b".to_owned(), HEXLOWER.encode(&tombstone.id.0)]],
            BASE64URL_NOPAD.encode(&frame.to_bytes()),
        );
        let _ = self.outgoing.send(json!(["EVENT", event]).to_string());
    }

    /// Posts what a handled frame should put on the internet: the tombstone
    /// of a bundle delivered to us, and, as a gateway, a bundle we started
    /// carrying from another link.
    pub(crate) fn post_handled(&self, handled: &Handled) {
        for frame in &handled.relay {
            match (handled.outcome, frame) {
                (Outcome::Received, Frame::Tombstone { .. }) => self.post_tombstone(frame),
                (Outcome::Carried, Frame::Bundle(bundle) | Frame::Chunk(bundle)) => {
                    self.post_bundle(frame, &bundle.tag)
                }
                _ => {}
            }
        }
    }
}

pub(crate) fn start(shared: &Arc<Shared>, config: InternetConfig) -> Result<()> {
    let secret = shared.engine().nostr_secret()?;
    if shared.internet.secret.set(secret).is_err() {
        return Err(Error::Invalid("the internet link is already running"));
    }
    let over_tor = config.tor.is_some();
    if let Some(dir) = config.tor {
        set_tor_state(shared, TorState::Starting);
        tokio::spawn(tor_loop(Arc::downgrade(shared), dir));
    }
    for relay in config.relays {
        tokio::spawn(relay_loop(
            Arc::downgrade(shared),
            relay,
            config.keepalive,
            over_tor,
        ));
    }
    Ok(())
}

/// Starts Tor, trying again until it works.
async fn tor_loop(shared: Weak<Shared>, dir: PathBuf) {
    loop {
        let started = async {
            let config =
                TorClientConfigBuilder::from_directories(dir.join("state"), dir.join("cache"))
                    .build()
                    .map_err(|error| error.to_string())?;
            TorClient::create_bootstrapped(config)
                .await
                .map_err(|error| error.to_string())
        }
        .await;
        let Some(shared) = shared.upgrade() else {
            return;
        };
        match started {
            Ok(client) => {
                set_tor_state(&shared, TorState::Ready);
                shared.internet.tor.send_replace(Some(Arc::new(client)));
                return;
            }
            Err(detail) => set_tor_state(&shared, TorState::Failed { detail }),
        }
        drop(shared);
        tokio::time::sleep(TOR_RETRY).await;
    }
}

fn set_tor_state(shared: &Shared, state: TorState) {
    *shared
        .internet
        .tor_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(state);
    shared.emit(Event::InternetChanged {
        relays: shared.internet.connected(),
    });
}

/// Opens a Tor stream to the relay's host, on a circuit of its own. The
/// host name is looked up at the exit, so it does not leak locally either.
async fn tor_stream(tor: &Tor, url: &str) -> Option<arti_client::DataStream> {
    tor.isolated_client()
        .connect(host_and_port(url)?)
        .await
        .ok()
}

/// The host and port a relay URL points at.
fn host_and_port(url: &str) -> Option<(String, u16)> {
    let uri: Uri = url.parse().ok()?;
    let host = uri.host()?.to_owned();
    let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("ws") {
        80
    } else {
        443
    });
    Some((host, port))
}

/// The id and `EVENT` message of the event that posts a bundle or chunk
/// frame under `tag`.
fn bundle_event(secret: &[u8; 32], frame: &Frame, tag: &RecipientTag) -> Option<(String, String)> {
    let (Frame::Bundle(bundle) | Frame::Chunk(bundle)) = frame else {
        return None;
    };
    let event = signed_event(
        secret,
        bundle.expires_at.saturating_sub(MAX_TTL_SECS),
        vec![vec!["x".to_owned(), HEXLOWER.encode(tag)]],
        BASE64URL_NOPAD.encode(&frame.to_bytes()),
    );
    let id = event["id"].as_str().unwrap_or_default().to_owned();
    Some((id, json!(["EVENT", event]).to_string()))
}

/// A Nostr event (NIP-01) signed with a one-time key derived from `secret`
/// and the content, so the same content always gives the same event.
pub fn signed_event(
    secret: &[u8; 32],
    created_at: u32,
    tags: Vec<Vec<String>>,
    content: String,
) -> Value {
    let mut seed: [u8; 32] = {
        let mut mac =
            <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("HMAC accepts any key length");
        mac.update(content.as_bytes());
        mac.finalize().into_bytes().into()
    };
    let key = loop {
        match SigningKey::from_bytes(&seed) {
            Ok(key) => break key,
            // Not a valid scalar, with odds of about 2^-128: hash again.
            Err(_) => seed = Sha256::digest(seed).into(),
        }
    };
    let pubkey = HEXLOWER.encode(&key.verifying_key().to_bytes());
    let serialized = json!([0, pubkey, created_at, KIND, tags, content]).to_string();
    let id: [u8; 32] = Sha256::digest(serialized.as_bytes()).into();
    let signature = key
        .sign_raw(&id, &[0; 32])
        .expect("signing a 32-byte digest works");
    json!({
        "id": HEXLOWER.encode(&id),
        "pubkey": pubkey,
        "created_at": created_at,
        "kind": KIND,
        "tags": tags,
        "content": content,
        "sig": HEXLOWER.encode(&signature.to_bytes()),
    })
}

async fn relay_loop(shared: Weak<Shared>, url: String, keepalive: Duration, over_tor: bool) {
    // A number for the relay, to count the relays that took a message.
    let relay = {
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        hasher.finish()
    };
    let mut retry = FIRST_RETRY;
    loop {
        let Some((mut outgoing, mut chunks, mut renew, mut probe, mut tor)) =
            shared.upgrade().map(|shared| {
                (
                    shared.internet.outgoing.subscribe(),
                    shared.internet.chunks.subscribe(),
                    shared.internet.renew.subscribe(),
                    shared.internet.probe.subscribe(),
                    shared.internet.tor.subscribe(),
                )
            })
        else {
            return;
        };
        let mut channels = Channels {
            outgoing: &mut outgoing,
            chunks: &mut chunks,
            renew: &mut renew,
            probe: &mut probe,
        };
        let ran = if over_tor {
            // Never directly: wait for Tor, however long it takes.
            let Ok(client) = tor
                .wait_for(Option::is_some)
                .await
                .map(|client| client.clone())
            else {
                return;
            };
            let client = client.expect("waited for a client");
            let connected = tokio::time::timeout(TOR_CONNECT_TIMEOUT, async {
                let stream = tor_stream(&client, &url).await?;
                tokio_tungstenite::client_async_tls(url.as_str(), stream)
                    .await
                    .ok()
            })
            .await;
            match connected {
                Ok(Some((socket, _))) => {
                    run_relay(
                        &shared,
                        relay,
                        socket,
                        &mut channels,
                        keepalive,
                        TOR_PROBE_TIMEOUT,
                    )
                    .await;
                    true
                }
                _ => false,
            }
        } else {
            let connected = tokio::time::timeout(
                CONNECT_TIMEOUT,
                tokio_tungstenite::connect_async(url.as_str()),
            )
            .await;
            match connected {
                Ok(Ok((socket, _))) => {
                    run_relay(
                        &shared,
                        relay,
                        socket,
                        &mut channels,
                        keepalive,
                        PROBE_TIMEOUT,
                    )
                    .await;
                    true
                }
                _ => false,
            }
        };
        if ran {
            retry = FIRST_RETRY;
        }
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(MAX_RETRY);
    }
}

/// What a relay connection listens to besides the relay.
struct Channels<'a> {
    outgoing: &'a mut broadcast::Receiver<String>,
    chunks: &'a mut broadcast::Receiver<String>,
    renew: &'a mut watch::Receiver<u64>,
    probe: &'a mut watch::Receiver<u64>,
}

async fn run_relay<S>(
    shared: &Weak<Shared>,
    relay: u64,
    socket: S,
    channels: &mut Channels<'_>,
    keepalive: Duration,
    probe_timeout: Duration,
) where
    S: futures_util::Sink<Message>
        + futures_util::Stream<Item = tokio_tungstenite::tungstenite::Result<Message>>
        + Unpin,
{
    let (mut sink, mut stream) = socket.split();
    let Some((greeting, own_chunks)) = with(shared, |shared| {
        shared.internet.connected.fetch_add(1, Ordering::Relaxed);
        shared.emit(Event::InternetChanged {
            relays: shared.internet.connected(),
        });
        greeting(shared)
    }) else {
        return;
    };

    let mut alive = true;
    for message in greeting {
        if sink.send(Message::text(message)).await.is_err() {
            alive = false;
            break;
        }
    }
    // Chunks wait their turn here, one sent every CHUNK_GAP.
    let mut pending: std::collections::VecDeque<String> = own_chunks.into();
    let mut pace = tokio::time::interval(CHUNK_GAP);
    pace.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut ping = tokio::time::interval(keepalive);
    let mut last_heard = Instant::now();
    // A probe asked for before or during the greeting still counts.
    let mut probe_deadline: Option<tokio::time::Instant> = None;
    let mut renewal = tokio::time::interval(RENEW_INTERVAL);
    // Both intervals fire at once; the greeting already covered that moment.
    ping.tick().await;
    renewal.tick().await;
    while alive {
        let send = tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(message)) => {
                    last_heard = Instant::now();
                    probe_deadline = None;
                    if let Message::Text(text) = message {
                        with(shared, |shared| receive(shared, relay, &text));
                    }
                    None
                }
            },
            message = channels.outgoing.recv() => match message {
                Ok(message) => Some(Message::text(message)),
                Err(broadcast::error::RecvError::Lagged(_)) => None,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            chunk = channels.chunks.recv() => match chunk {
                Ok(chunk) => {
                    pending.push_back(chunk);
                    None
                }
                // Missed chunks go out again on the next connection.
                Err(broadcast::error::RecvError::Lagged(_)) => None,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = pace.tick(), if !pending.is_empty() => pending.pop_front().map(Message::text),
            changed = channels.renew.changed() => match changed {
                Ok(()) => with(shared, subscription).flatten().map(Message::text),
                Err(_) => break,
            },
            _ = renewal.tick() => with(shared, subscription).flatten().map(Message::text),
            changed = channels.probe.changed() => match changed {
                Ok(()) => {
                    probe_deadline.get_or_insert(tokio::time::Instant::now() + probe_timeout);
                    Some(Message::Ping(Default::default()))
                }
                Err(_) => break,
            },
            // Asked to prove itself and stayed silent: dead.
            _ = tokio::time::sleep_until(
                probe_deadline.unwrap_or_else(|| tokio::time::Instant::now() + RENEW_INTERVAL)
            ), if probe_deadline.is_some() => break,
            _ = ping.tick() => {
                // Not even a pong for two rounds: the connection is dead,
                // even if the computer has not noticed yet.
                if last_heard.elapsed() > keepalive * 2 {
                    break;
                }
                Some(Message::Ping(Default::default()))
            }
        };
        if let Some(message) = send {
            alive = sink.send(message).await.is_ok();
        }
    }

    with(shared, |shared| {
        shared.internet.connected.fetch_sub(1, Ordering::Relaxed);
        shared.emit(Event::InternetChanged {
            relays: shared.internet.connected(),
        });
    });
}

/// What a node sends a relay it just reached: its subscription, its own
/// bundles still waiting for a receipt, and recent receipts it gave. Its own
/// file chunks come apart, to be sent at a measured pace.
fn greeting(shared: &Shared) -> (Vec<String>, Vec<String>) {
    let mut messages: Vec<String> = subscription(shared).into_iter().collect();
    let mut chunks = Vec::new();
    let mut engine = shared.engine();
    let secret = shared.internet.secret.get().copied();
    let Some(secret) = secret else {
        return (messages, chunks);
    };
    let mut frames = engine.outbox(now()).unwrap_or_default();
    // What it carries for others and could not post while offline.
    frames.extend(engine.take_uploads(now()).unwrap_or_default());
    for (frame, tag) in frames {
        if let Some(event) = shared.internet.event_for(&secret, &frame, &tag) {
            match frame {
                Frame::Chunk(_) => chunks.push(event),
                _ => messages.push(event),
            }
        }
    }
    for frame in engine.received_tombstones().unwrap_or_default() {
        if let Frame::Tombstone { tombstone, .. } = &frame {
            let event = signed_event(
                &secret,
                now(),
                vec![vec!["b".to_owned(), HEXLOWER.encode(&tombstone.id.0)]],
                BASE64URL_NOPAD.encode(&frame.to_bytes()),
            );
            messages.push(json!(["EVENT", event]).to_string());
        }
    }
    (messages, chunks)
}

fn subscription(shared: &Shared) -> Option<String> {
    let (tags, waiting) = shared.engine().subscription(now()).ok()?;
    let since = now().saturating_sub(LOOKBACK_SECS);
    let tags: Vec<String> = tags.iter().map(|tag| HEXLOWER.encode(tag)).collect();
    let mut message = vec![
        json!("REQ"),
        json!(SUBSCRIPTION_ID),
        json!({"kinds": [KIND], "#x": tags, "since": since}),
    ];
    if !waiting.is_empty() {
        let ids: Vec<String> = waiting.iter().map(|id| HEXLOWER.encode(&id.0)).collect();
        message.push(json!({"kinds": [KIND], "#b": ids, "since": since}));
    }
    Some(Value::Array(message).to_string())
}

/// Handles one message from a relay.
/// Handles a message from the relay numbered `relay`.
fn receive(shared: &Shared, relay: u64, text: &str) {
    let Ok(Value::Array(parts)) = serde_json::from_str::<Value>(text) else {
        return;
    };
    match parts.first().and_then(Value::as_str) {
        Some("EVENT") => {
            let Some(event) = parts.get(2) else {
                return;
            };
            if event["kind"].as_u64() != Some(KIND.into()) {
                return;
            }
            let Some(frame) = event["content"]
                .as_str()
                .and_then(|content| BASE64URL_NOPAD.decode(content.as_bytes()).ok())
                .and_then(|bytes| Frame::from_bytes(&bytes).ok())
            else {
                return;
            };
            // Anyone can post to a relay: take only what is for us (§9.3).
            let handled = shared
                .engine()
                .atomically(|engine| engine.handle_with(frame, now(), false));
            if let Ok(handled) = handled {
                shared.dispatch(handled, None);
            }
        }
        Some("OK") => {
            let accepted = parts.get(2).and_then(Value::as_bool).unwrap_or(true);
            let carried = parts.get(1).and_then(Value::as_str).and_then(|id| {
                let posted = shared
                    .internet
                    .posted
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                posted.get(id).copied()
            });
            if let Some(bundle) = carried.filter(|_| accepted) {
                shared.handed_off_id(bundle, Via::Internet, relay);
            }
            let reason = parts.get(3).and_then(Value::as_str).unwrap_or_default();
            let mut refusal = shared
                .internet
                .refusal
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *refusal = (!accepted).then(|| reason.to_owned());
        }
        _ => {}
    }
}

fn with<T>(shared: &Weak<Shared>, f: impl FnOnce(&Shared) -> T) -> Option<T> {
    shared.upgrade().map(|shared| f(&shared))
}

#[cfg(test)]
mod tests {

    #[test]
    fn relay_urls_name_a_host_and_port() {
        assert_eq!(
            super::host_and_port("wss://nos.lol"),
            Some(("nos.lol".into(), 443))
        );
        assert_eq!(
            super::host_and_port("ws://10.0.0.2:7777"),
            Some(("10.0.0.2".into(), 7777))
        );
        assert_eq!(
            super::host_and_port("ws://relay.local"),
            Some(("relay.local".into(), 80))
        );
        assert_eq!(super::host_and_port("not a url"), None);
    }

    use super::*;
    use k256::schnorr::{Signature, VerifyingKey};

    #[test]
    fn events_are_valid_nip01_and_repeatable() {
        let secret = [7; 32];
        let tags = vec![vec!["x".to_owned(), "00112233aabbccdd".to_owned()]];
        let event = signed_event(&secret, 1_790_000_000, tags.clone(), "AQID".to_owned());
        assert_eq!(
            event,
            signed_event(&secret, 1_790_000_000, tags, "AQID".to_owned())
        );

        let serialized = format!(
            r#"[0,"{}",1790000000,9378,[["x","00112233aabbccdd"]],"AQID"]"#,
            event["pubkey"].as_str().unwrap()
        );
        let id: [u8; 32] = Sha256::digest(serialized.as_bytes()).into();
        assert_eq!(event["id"], HEXLOWER.encode(&id));

        let pubkey = HEXLOWER
            .decode(event["pubkey"].as_str().unwrap().as_bytes())
            .unwrap();
        let signature = HEXLOWER
            .decode(event["sig"].as_str().unwrap().as_bytes())
            .unwrap();
        VerifyingKey::from_bytes(&pubkey)
            .unwrap()
            .verify_raw(&id, &Signature::try_from(signature.as_slice()).unwrap())
            .unwrap();
    }

    #[test]
    fn different_content_gets_an_unrelated_key() {
        let a = signed_event(&[7; 32], 1, vec![], "a".to_owned());
        let b = signed_event(&[7; 32], 1, vec![], "b".to_owned());
        assert_ne!(a["pubkey"], b["pubkey"]);
    }
}

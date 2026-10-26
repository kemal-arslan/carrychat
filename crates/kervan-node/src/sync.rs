//! Syncing with a nearby node over a byte stream: a TCP connection on the
//! local network (§9.1), a Bluetooth LE connection (§9.5). Both sides say
//! HELLO, offer what they have, ask for what they lack, then push what is new.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use kervan_proto::bundle::BundleId;
use kervan_proto::frame::Frame;
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::{now, Error, Event, Result, Shared};

const HAVE: u8 = 0x10;
const WANT: u8 = 0x11;
const HELLO: u8 = 0x12;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_IDS_PER_FRAME: usize = 8000;
/// Sightings a node with the larger session id waits for the other to
/// connect before it connects itself, in case the other cannot.
const PATIENCE_SIGHTINGS: u32 = 3;

/// The link a nearby node is connected over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Link {
    Lan,
    Bluetooth,
}

type Outbox = mpsc::UnboundedSender<Vec<u8>>;

/// The nodes connected over any nearby link, by session id. One node is
/// connected once: a second connection to it, over any link, is closed.
pub(crate) struct Peers {
    session: u64,
    connected: Mutex<HashMap<u64, (Link, Outbox)>>,
    dialing: Mutex<HashSet<u64>>,
    patience: Mutex<HashMap<u64, u32>>,
}

impl Peers {
    pub(crate) fn new() -> Self {
        Self {
            session: OsRng.next_u64(),
            connected: Mutex::default(),
            dialing: Mutex::default(),
            patience: Mutex::default(),
        }
    }

    /// The id this run of the node goes by on every nearby link.
    pub(crate) fn session(&self) -> u64 {
        self.session
    }

    pub(crate) fn count(&self, link: Link) -> usize {
        lock(&self.connected)
            .values()
            .filter(|(connected, _)| *connected == link)
            .count()
    }

    pub(crate) fn broadcast(&self, frame: &[u8], except: Option<u64>) {
        for (session, (_, outbox)) in lock(&self.connected).iter() {
            if Some(*session) != except {
                let _ = outbox.send(frame.to_vec());
            }
        }
    }

    /// Whether to connect to a node just seen nearby. The node with the
    /// smaller session id connects; the other waits a few sightings first,
    /// unless the other `cannot_dial` it, as a desktop cannot over Bluetooth.
    /// A yes counts as a connection in progress until `dial_ended`.
    pub(crate) fn should_dial(&self, peer: u64, cannot_dial: bool) -> bool {
        if peer == self.session || lock(&self.connected).contains_key(&peer) {
            return false;
        }
        if self.session > peer && !cannot_dial {
            let mut patience = lock(&self.patience);
            let sightings = patience.entry(peer).or_insert(0);
            *sightings += 1;
            if *sightings < PATIENCE_SIGHTINGS {
                return false;
            }
        }
        lock(&self.dialing).insert(peer)
    }

    pub(crate) fn dial_ended(&self, peer: u64) {
        lock(&self.dialing).remove(&peer);
    }

    fn register(&self, session: u64, link: Link, outbox: &Outbox) -> bool {
        let mut connected = lock(&self.connected);
        if connected.contains_key(&session) {
            return false;
        }
        connected.insert(session, (link, outbox.clone()));
        lock(&self.patience).remove(&session);
        true
    }

    fn unregister(&self, session: u64, outbox: &Outbox) {
        let mut connected = lock(&self.connected);
        if connected
            .get(&session)
            .is_some_and(|(_, registered)| registered.same_channel(outbox))
        {
            connected.remove(&session);
        }
    }
}

/// Syncs over one connection until it ends. `dialed` is the session id of
/// the node this side connected to, if it connected to one it had seen.
pub(crate) async fn run<R, W>(
    shared: Arc<Shared>,
    link: Link,
    reader: R,
    writer: W,
    dialed: Option<u64>,
) where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut reader = BufReader::new(reader);
    let (outbox, queue) = mpsc::unbounded_channel();
    let writer = tokio::spawn(write_loop(writer, queue));

    let mut hello = vec![HELLO];
    hello.extend_from_slice(&shared.peers.session.to_be_bytes());
    let _ = outbox.send(hello);
    let handshake = tokio::time::timeout(READ_TIMEOUT, read_frame(&mut reader)).await;
    if let Some(peer) = dialed {
        shared.peers.dial_ended(peer);
    }
    let peer = match handshake {
        Ok(Ok(frame)) => parse_hello(&frame),
        _ => None,
    };
    let Some(peer) = peer.filter(|&peer| shared.peers.register(peer, link, &outbox)) else {
        writer.abort();
        return;
    };
    shared.peers_changed();

    if let Ok(frames) = initial_sync(&shared) {
        for frame in frames {
            let _ = outbox.send(frame);
        }
    }
    let heartbeat = tokio::spawn(heartbeat(outbox.clone()));
    while let Ok(Ok(frame)) = tokio::time::timeout(READ_TIMEOUT, read_frame(&mut reader)).await {
        // A frame that does not parse is skipped, as on every link.
        let _ = handle_frame(&shared, peer, &outbox, &frame);
    }

    heartbeat.abort();
    writer.abort();
    shared.peers.unregister(peer, &outbox);
    shared.peers_changed();
}

impl Shared {
    fn peers_changed(&self) {
        self.emit(Event::PeersChanged {
            lan: self.peers.count(Link::Lan),
            bluetooth: self.peers.count(Link::Bluetooth),
        });
    }
}

/// What a node sends right after the handshake: the ids of the bundles it can
/// pass on, and every tombstone it knows.
fn initial_sync(shared: &Shared) -> Result<Vec<Vec<u8>>> {
    let engine = shared.engine();
    let mut frames: Vec<Vec<u8>> = engine
        .offer(now())?
        .chunks(MAX_IDS_PER_FRAME)
        .map(|ids| ids_frame(HAVE, ids))
        .collect();
    frames.extend(engine.tombstone_frames()?.iter().map(Frame::to_bytes));
    Ok(frames)
}

fn handle_frame(shared: &Shared, peer: u64, outbox: &Outbox, frame: &[u8]) -> Result<()> {
    match frame.first() {
        Some(&HAVE) => {
            let missing = shared.engine().missing(&parse_ids(frame)?)?;
            for ids in missing.chunks(MAX_IDS_PER_FRAME) {
                let _ = outbox.send(ids_frame(WANT, ids));
            }
        }
        Some(&WANT) => {
            for frame in shared.engine().frames_for(&parse_ids(frame)?, now())? {
                let _ = outbox.send(frame.to_bytes());
            }
        }
        Some(&HELLO) => {}
        _ => {
            let handled = shared.handle(Frame::from_bytes(frame)?)?;
            shared.dispatch(handled, Some(peer));
        }
    }
    Ok(())
}

/// An empty offer every so often, so a peer that vanished times out.
async fn heartbeat(outbox: Outbox) {
    let mut tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        tick.tick().await;
        if outbox.send(ids_frame(HAVE, &[])).is_err() {
            return;
        }
    }
}

async fn write_loop<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut queue: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Some(frame) = queue.recv().await {
        let Ok(len) = u16::try_from(frame.len()) else {
            continue;
        };
        let mut bytes = len.to_be_bytes().to_vec();
        bytes.extend_from_slice(&frame);
        if writer.write_all(&bytes).await.is_err() || writer.flush().await.is_err() {
            return;
        }
    }
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Vec<u8>> {
    let len = reader.read_u16().await?;
    let mut frame = vec![0; len as usize];
    reader.read_exact(&mut frame).await?;
    Ok(frame)
}

fn ids_frame(frame_type: u8, ids: &[BundleId]) -> Vec<u8> {
    let mut frame = vec![frame_type];
    frame.extend_from_slice(&(ids.len() as u16).to_be_bytes());
    for id in ids {
        frame.extend_from_slice(&id.0);
    }
    frame
}

fn parse_ids(frame: &[u8]) -> Result<Vec<BundleId>> {
    let count = frame
        .get(1..3)
        .map(|count| u16::from_be_bytes([count[0], count[1]]) as usize)
        .ok_or(Error::Invalid("id list"))?;
    let ids = &frame[3..];
    if ids.len() != count * 8 {
        return Err(Error::Invalid("id list"));
    }
    Ok(ids
        .chunks_exact(8)
        .map(|id| BundleId(id.try_into().expect("chunks are 8 bytes")))
        .collect())
}

fn parse_hello(frame: &[u8]) -> Option<u64> {
    match frame {
        [HELLO, session @ ..] => Some(u64::from_be_bytes(session.try_into().ok()?)),
        _ => None,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

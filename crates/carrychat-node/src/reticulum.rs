//! The Reticulum link (§9.9): the node connects over TCP to a Reticulum
//! node, as a plain instance that does not route for others, announces a
//! destination of its own, `carrychat.frames`, and sends each frame
//! encrypted to every other CarryChat destination announced on the
//! network, however many hops away. The wire format is in `wire`.

pub mod wire;

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use carrychat_proto::frame::{frames_for, Frame};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;
use tokio::task::AbortHandle;

use self::wire::{Announce, Identity, Packet, PacketType};
use crate::{now, Event, Result, Shared};

const RECONNECT_DELAY: Duration = Duration::from_secs(10);
/// How often the node announces itself while connected, besides on every
/// connection. Transport nodes stop passing on the announces of a
/// destination that announces more often than hourly.
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Between path requests, under the rate transport nodes accept from a new
/// client.
const PATH_REQUEST_GAP: Duration = Duration::from_millis(500);
/// Destinations kept across restarts.
const MAX_KEPT_PEERS: usize = 64;
/// A node that hears a CarryChat node it did not know announces itself
/// again, so the newcomer learns of it too: up to this many at once, then
/// one more every `WELCOME_REFILL`, about what transport nodes pass on
/// before they hold a destination's announces back. A welcome that has to
/// wait is made late, not dropped.
const WELCOME_BURST: f64 = 5.0;
const WELCOME_REFILL: Duration = Duration::from_secs(10 * 60);
/// Receipts for the latest messages delivered to us that go to a CarryChat
/// node newly heard, with our own messages still waiting for a receipt.
const PENDING_RECEIPTS: usize = 32;
/// How long an announced CarryChat destination counts as there.
const PEER_WITHIN_SECS: u32 = 6 * 60 * 60;
/// Frames waiting to go out. Past these, the oldest carried one goes first,
/// then the oldest own one.
const MAX_CARRIED: usize = 64;
const MAX_OWN: usize = 256;
/// Packet hashes kept to drop what arrives twice.
const MAX_SEEN_PACKETS: usize = 1024;

/// How the Reticulum link stands, for the app.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReticulumStatus {
    /// The node connected to, as `host:port`.
    pub address: Option<String>,
    pub connected: bool,
    /// This node's own destination, in hex.
    pub destination: Option<String>,
    /// Other CarryChat destinations announced in the last six hours, latest
    /// first.
    pub peers: Vec<ReticulumPeer>,
    /// Frames waiting to go out.
    pub waiting: usize,
    /// When CarryChat last came in over Reticulum, in seconds since the
    /// epoch.
    pub carrychat_at: Option<u32>,
}

/// A CarryChat destination announced on the network.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReticulumPeer {
    /// Its destination, in hex.
    pub destination: String,
    /// Hops between this node and it.
    pub hops: u8,
    /// When it last announced itself, in seconds since the epoch.
    pub at: u32,
}

/// Another CarryChat destination, as its announce gave it.
struct Peer {
    announce: Announce,
    at: u32,
}

#[derive(Default)]
struct Seen {
    own: Option<[u8; wire::HASH_LEN]>,
    peers: HashMap<[u8; wire::HASH_LEN], Peer>,
    carrychat_at: Option<u32>,
}

#[derive(Default)]
struct Queue {
    own: VecDeque<Vec<u8>>,
    carried: VecDeque<Vec<u8>>,
    /// Frames for one destination only: what waited for a node newly heard.
    backlog: VecDeque<([u8; wire::HASH_LEN], Vec<u8>)>,
}

pub(crate) struct Reticulum {
    started: AtomicBool,
    connected: AtomicBool,
    address: Mutex<Option<String>>,
    task: Mutex<Option<AbortHandle>>,
    queue: Mutex<Queue>,
    wake: Arc<Notify>,
    seen: Mutex<Seen>,
}

impl Reticulum {
    pub(crate) fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            connected: AtomicBool::new(false),
            address: Mutex::new(None),
            task: Mutex::new(None),
            queue: Mutex::default(),
            wake: Arc::new(Notify::new()),
            seen: Mutex::default(),
        }
    }

    pub(crate) fn status(&self, now: u32) -> ReticulumStatus {
        let seen = lock(&self.seen);
        let mut peers: Vec<ReticulumPeer> = seen
            .peers
            .iter()
            .filter(|(_, peer)| now.saturating_sub(peer.at) <= PEER_WITHIN_SECS)
            .map(|(destination, peer)| ReticulumPeer {
                destination: hex(destination),
                // The node this one connects to is a hop too.
                hops: peer.announce.hops.saturating_add(1),
                at: peer.at,
            })
            .collect();
        peers.sort_by_key(|peer| std::cmp::Reverse(peer.at));
        let queue = lock(&self.queue);
        ReticulumStatus {
            address: lock(&self.address).clone(),
            connected: self.connected(),
            destination: seen.own.as_ref().map(|own| hex(own)),
            peers,
            waiting: queue.own.len() + queue.carried.len() + queue.backlog.len(),
            carrychat_at: seen.carrychat_at,
        }
    }

    pub(crate) fn connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Queues a frame for every CarryChat destination known. Only bundles
    /// and tombstones go over Reticulum, never file chunks. `own` frames go
    /// before ones carried for others. Returns whether it was queued while
    /// connected, with someone to send it to.
    pub(crate) fn send(&self, frame: &Frame, own: bool) -> bool {
        if !self.started.load(Ordering::Relaxed) {
            return false;
        }
        let packets = packets(frame);
        if packets.is_empty() {
            return false;
        }
        {
            let mut queue = lock(&self.queue);
            let own = own || matches!(frame, Frame::Tombstone { .. });
            let (lane, max) = if own {
                (&mut queue.own, MAX_OWN)
            } else {
                (&mut queue.carried, MAX_CARRIED)
            };
            lane.extend(packets);
            while lane.len() > max {
                lane.pop_front();
            }
        }
        self.wake.notify_one();
        self.connected() && !self.peers(now()).is_empty()
    }

    /// The CarryChat destinations announced lately.
    fn peers(&self, now: u32) -> Vec<Announce> {
        lock(&self.seen)
            .peers
            .values()
            .filter(|peer| now.saturating_sub(peer.at) <= PEER_WITHIN_SECS)
            .map(|peer| peer.announce.clone())
            .collect()
    }
}

/// A frame as the packets that carry it over Reticulum: a bundle, whole or
/// in fragments, or a tombstone. File chunks do not go over Reticulum.
fn packets(frame: &Frame) -> Vec<Vec<u8>> {
    match frame {
        Frame::Bundle(bundle) => frames_for(bundle, wire::MTU)
            .map(|frames| frames.iter().map(Frame::to_bytes).collect())
            .unwrap_or_default(),
        Frame::Tombstone { .. } => vec![frame.to_bytes()],
        Frame::Fragment(_) | Frame::Chunk(_) => Vec::new(),
    }
}

/// Welcomes this node may still make: up to `WELCOME_BURST`, refilled at
/// one every `WELCOME_REFILL`.
struct Welcomes {
    left: f64,
    at: tokio::time::Instant,
}

impl Welcomes {
    fn new() -> Self {
        Self {
            left: WELCOME_BURST,
            at: tokio::time::Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = tokio::time::Instant::now();
        let refilled = now.duration_since(self.at).as_secs_f64() / WELCOME_REFILL.as_secs_f64();
        self.left = (self.left + refilled).min(WELCOME_BURST);
        self.at = now;
    }

    /// How long until a welcome may be made.
    fn wait(&mut self) -> Duration {
        self.refill();
        if self.left >= 1.0 {
            Duration::ZERO
        } else {
            WELCOME_REFILL.mul_f64(1.0 - self.left)
        }
    }

    fn spend(&mut self) {
        self.refill();
        self.left -= 1.0;
    }
}

/// Connects to the Reticulum node at `address` from now on, in place of any
/// other, and again whenever the connection ends.
pub(crate) fn start(shared: &Arc<Shared>, address: String) -> Result<()> {
    stop(shared);
    let (secret, known) = {
        let engine = shared.engine();
        (engine.reticulum_secret()?, engine.reticulum_peers()?)
    };
    let identity = Identity::from_secret(&secret);
    let reticulum = &shared.reticulum;
    {
        let mut seen = lock(&reticulum.seen);
        seen.own = Some(identity.destination());
        seen.peers = unpack_peers(&known);
    }
    *lock(&reticulum.address) = Some(address.clone());
    reticulum.started.store(true, Ordering::Relaxed);
    let task = tokio::spawn(run(Arc::downgrade(shared), address, identity));
    *lock(&reticulum.task) = Some(task.abort_handle());
    Ok(())
}

/// Stops using Reticulum, and forgets what was heard.
pub(crate) fn stop(shared: &Shared) {
    let reticulum = &shared.reticulum;
    if let Some(task) = lock(&reticulum.task).take() {
        task.abort();
    }
    reticulum.started.store(false, Ordering::Relaxed);
    *lock(&reticulum.address) = None;
    *lock(&reticulum.queue) = Queue::default();
    *lock(&reticulum.seen) = Seen::default();
    if reticulum.connected.swap(false, Ordering::Relaxed) {
        shared.emit(Event::ReticulumChanged { connected: false });
    }
}

async fn run(shared: Weak<Shared>, address: String, identity: Identity) {
    // Each announce must be later than the last, or nodes that know this
    // destination ignore it.
    let mut announced = 0;
    loop {
        let result = match tokio::net::TcpStream::connect(&address).await {
            Ok(stream) => session(&shared, stream, &identity, &mut announced).await,
            Err(error) => Err(error),
        };
        let Some(node) = shared.upgrade() else {
            return;
        };
        if node.reticulum.connected.swap(false, Ordering::Relaxed) {
            node.emit(Event::ReticulumChanged { connected: false });
        }
        if let Err(error) = result {
            eprintln!("Reticulum: {error}");
        }
        drop(node);
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// An announce of this node's destination, later than the last one.
fn announce(identity: &Identity, announced: &mut u64, path_response: bool) -> Vec<u8> {
    *announced = u64::from(now()).max(*announced + 1);
    wire::frame(&wire::announce(identity, *announced, path_response))
}

/// One connection: announces this node, asks for the paths to the CarryChat
/// nodes it knows, since nodes do not pass on announces made before a
/// client connected, then sends from the queue to every destination heard
/// and takes in what comes for this one.
async fn session(
    shared: &Weak<Shared>,
    stream: tokio::net::TcpStream,
    identity: &Identity,
    announced: &mut u64,
) -> io::Result<()> {
    let _ = stream.set_nodelay(true);
    let (mut reader, mut writer) = stream.into_split();
    let own = identity.destination();
    let Some(node) = shared.upgrade() else {
        return Ok(());
    };
    let wake = Arc::clone(&node.reticulum.wake);
    let mut asking: VecDeque<[u8; wire::HASH_LEN]> =
        lock(&node.reticulum.seen).peers.keys().copied().collect();
    node.reticulum.connected.store(true, Ordering::Relaxed);
    node.emit(Event::ReticulumChanged { connected: true });
    drop(node);
    writer
        .write_all(&announce(identity, announced, false))
        .await?;

    let mut deframer = wire::Deframer::default();
    let mut seen_packets: VecDeque<[u8; 32]> = VecDeque::new();
    let mut seen_set: HashSet<[u8; 32]> = HashSet::new();
    let start = tokio::time::Instant::now();
    let mut announce_timer = tokio::time::interval_at(start + ANNOUNCE_INTERVAL, ANNOUNCE_INTERVAL);
    let mut ask_timer = tokio::time::interval(PATH_REQUEST_GAP);
    let mut welcomes = Welcomes::new();
    let mut welcome_due = false;
    let mut buffer = vec![0; 4096];
    loop {
        let welcome_in = welcome_due.then(|| welcomes.wait());
        if welcome_in == Some(Duration::ZERO) {
            welcomes.spend();
            welcome_due = false;
            writer
                .write_all(&announce(identity, announced, false))
                .await?;
            continue;
        }
        // What waited for a node newly heard goes to it alone.
        let backlog = shared.upgrade().and_then(|node| {
            let next = lock(&node.reticulum.queue).backlog.pop_front()?;
            let peer = lock(&node.reticulum.seen)
                .peers
                .get(&next.0)?
                .announce
                .clone();
            Some((peer, next.1))
        });
        if let Some((peer, frame)) = backlog {
            writer
                .write_all(&wire::frame(&wire::data_packet(&peer, &frame)))
                .await?;
            continue;
        }
        // Every frame waiting goes to every destination heard lately.
        let peers = shared
            .upgrade()
            .map(|node| node.reticulum.peers(now()))
            .unwrap_or_default();
        if !peers.is_empty() {
            let frame = shared.upgrade().and_then(|node| {
                let mut queue = lock(&node.reticulum.queue);
                queue.own.pop_front().or_else(|| queue.carried.pop_front())
            });
            if let Some(frame) = frame {
                for peer in &peers {
                    let packet = wire::data_packet(peer, &frame);
                    writer.write_all(&wire::frame(&packet)).await?;
                }
                continue;
            }
        }
        tokio::select! {
            read = reader.read(&mut buffer) => {
                let len = read?;
                if len == 0 {
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                }
                deframer.push(&buffer[..len]);
                while let Some(raw) = deframer.next_frame() {
                    let Some(packet) = Packet::parse(&raw) else { continue };
                    let hash = packet.hash();
                    if !seen_set.insert(hash) {
                        continue;
                    }
                    seen_packets.push_back(hash);
                    if seen_packets.len() > MAX_SEEN_PACKETS {
                        if let Some(old) = seen_packets.pop_front() {
                            seen_set.remove(&old);
                        }
                    }
                    match take(shared, identity, &own, &packet) {
                        Reply::Nothing => {}
                        Reply::PathResponse => {
                            writer.write_all(&announce(identity, announced, true)).await?;
                        }
                        Reply::Welcome => welcome_due = true,
                    }
                }
            }
            () = wake.notified() => {}
            () = tokio::time::sleep(welcome_in.unwrap_or_default()), if welcome_in.is_some() => {}
            _ = announce_timer.tick() => {
                writer.write_all(&announce(identity, announced, false)).await?;
            }
            _ = ask_timer.tick(), if !asking.is_empty() => {
                if let Some(destination) = asking.pop_front() {
                    let mut tag = [0; 16];
                    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut tag);
                    writer.write_all(&wire::frame(&wire::path_request(&destination, &tag))).await?;
                }
            }
        }
    }
}

/// What a packet taken in calls for.
enum Reply {
    Nothing,
    /// It asked for this node's path, which an announce answers.
    PathResponse,
    /// It announced a CarryChat node this one did not know.
    Welcome,
}

/// Takes in a packet: an announce of another CarryChat destination, a frame
/// for this one, or a request for its path.
fn take(
    shared: &Weak<Shared>,
    identity: &Identity,
    own: &[u8; wire::HASH_LEN],
    packet: &Packet,
) -> Reply {
    let Some(node) = shared.upgrade() else {
        return Reply::Nothing;
    };
    match packet.packet_type {
        PacketType::Announce => {
            let Some(announce) = Announce::validate(packet) else {
                return Reply::Nothing;
            };
            if announce.destination == *own || announce.name_hash != wire::name_hash(wire::NAME) {
                return Reply::Nothing;
            }
            let mut seen = lock(&node.reticulum.seen);
            // A destination keeps the key it was first announced with.
            if seen
                .peers
                .get(&announce.destination)
                .is_some_and(|peer| peer.announce.public_key != announce.public_key)
            {
                return Reply::Nothing;
            }
            let at = now();
            let new = !seen.peers.contains_key(&announce.destination);
            // Not heard in this run yet, or not lately: it may have missed
            // what we sent meanwhile, or before the link started.
            let fresh = seen
                .peers
                .get(&announce.destination)
                .is_none_or(|peer| at.saturating_sub(peer.at) > PEER_WITHIN_SECS);
            let destination = announce.destination;
            seen.peers.insert(destination, Peer { announce, at });
            let packed = new.then(|| pack_peers(&seen.peers));
            drop(seen);
            if fresh {
                let pending = node.engine().pending_own(at, PENDING_RECEIPTS);
                let mut queue = lock(&node.reticulum.queue);
                for frame in pending.unwrap_or_default() {
                    for packet in packets(&frame) {
                        queue.backlog.push_back((destination, packet));
                    }
                }
            }
            node.reticulum.wake.notify_one();
            match packed {
                Some(packed) => {
                    let _ = node.engine().set_reticulum_peers(&packed);
                    Reply::Welcome
                }
                None => Reply::Nothing,
            }
        }
        PacketType::Data if packet.destination == *own => {
            let Some(bytes) = identity.decrypt(&packet.data) else {
                return Reply::Nothing;
            };
            // A frame that does not parse is skipped, as on every link; file
            // chunks never belong on Reticulum.
            let frame = match Frame::from_bytes(&bytes) {
                Ok(Frame::Chunk(_)) | Err(_) => return Reply::Nothing,
                Ok(frame) => frame,
            };
            lock(&node.reticulum.seen).carrychat_at = Some(now());
            if let Ok(handled) = node.handle(frame.clone()) {
                node.dispatch_from_reticulum(handled, &frame);
            }
            Reply::Nothing
        }
        PacketType::Data if wire::requested_path(packet) == Some(*own) => Reply::PathResponse,
        _ => Reply::Nothing,
    }
}

/// The destinations heard, latest first and at most `MAX_KEPT_PEERS`, as
/// each destination's hash followed by its public key.
fn pack_peers(peers: &HashMap<[u8; wire::HASH_LEN], Peer>) -> Vec<u8> {
    let mut latest: Vec<&Peer> = peers.values().collect();
    latest.sort_by_key(|peer| std::cmp::Reverse(peer.at));
    latest
        .into_iter()
        .take(MAX_KEPT_PEERS)
        .flat_map(|peer| [&peer.announce.destination[..], &peer.announce.public_key].concat())
        .collect()
}

/// Destinations kept from before, not heard yet: nothing is sent to them
/// until an answer to a path request says where they are.
fn unpack_peers(bytes: &[u8]) -> HashMap<[u8; wire::HASH_LEN], Peer> {
    bytes
        .chunks_exact(wire::HASH_LEN + 64)
        .filter_map(|record| {
            let destination: [u8; wire::HASH_LEN] = record[..wire::HASH_LEN].try_into().ok()?;
            let public_key: [u8; 64] = record[wire::HASH_LEN..].try_into().ok()?;
            let announce = Announce {
                destination,
                public_key,
                name_hash: wire::name_hash(wire::NAME),
                hops: 0,
                transport_id: None,
                emitted: 0,
            };
            Some((destination, Peer { announce, at: 0 }))
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

//! The local network link (§9.1): nodes find each other with UDP multicast
//! beacons, then sync bundles and tombstones over TCP.

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use kervan_proto::bundle::BundleId;
use kervan_proto::frame::Frame;
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use socket2::{Domain, Protocol, SockRef, Socket, Type};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc;

use crate::{now, Error, Event, Result, Shared};

pub const DEFAULT_GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 74, 74), 47474);

const HAVE: u8 = 0x10;
const WANT: u8 = 0x11;
const HELLO: u8 = 0x12;
const BEACON_PREFIX: &[u8; 6] = b"KRVN\x00\x01";
const BEACON_LEN: usize = 16;
const BEACON_INTERVAL: Duration = Duration::from_secs(3);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(45);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_IDS_PER_FRAME: usize = 8000;
/// The node with the smaller session id dials. The other one dials too after
/// this many beacons without a connection, in case its beacons do not arrive.
const PATIENCE_BEACONS: u32 = 3;
/// Beacons that may fail in a row before the discovery socket is replaced,
/// for example after the computer switched networks.
const MAX_BEACON_FAILURES: u32 = 3;
/// How long a failed connection to a discovered node keeps showing as a problem.
const DIAL_PROBLEM_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct LanConfig {
    /// Multicast group and port for discovery. `None` turns discovery off,
    /// leaving incoming and [`crate::Node::connect`] connections.
    pub discovery: Option<SocketAddrV4>,
    /// TCP port to listen on. 0 picks a free one.
    pub listen_port: u16,
}

impl Default for LanConfig {
    fn default() -> Self {
        Self {
            discovery: Some(DEFAULT_GROUP),
            listen_port: 0,
        }
    }
}

/// Why the local network link is not working, for the status bar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LanProblem {
    /// The operating system does not let Kervan use the local network. On
    /// macOS this is the Local Network permission in Privacy & Security.
    Blocked { detail: String },
    /// Something else failed, such as a firewall refusing a connection.
    Failed { detail: String },
}

impl LanProblem {
    fn from_error(context: String, error: &io::Error) -> Self {
        let detail = format!("{context}: {error}");
        if blocked_by_os(error) {
            Self::Blocked { detail }
        } else {
            Self::Failed { detail }
        }
    }
}

/// Without the Local Network permission, macOS answers "no route to host" or
/// "operation not permitted" to everything sent on the local network.
fn blocked_by_os(error: &io::Error) -> bool {
    cfg!(target_os = "macos")
        && matches!(
            error.kind(),
            io::ErrorKind::HostUnreachable | io::ErrorKind::PermissionDenied
        )
}

#[derive(Default)]
struct Problems {
    beacon: Option<LanProblem>,
    dial: Option<(LanProblem, Instant)>,
}

impl Problems {
    fn current(&self) -> Option<LanProblem> {
        self.beacon.clone().or_else(|| {
            self.dial
                .as_ref()
                .filter(|(_, at)| at.elapsed() < DIAL_PROBLEM_TTL)
                .map(|(problem, _)| problem.clone())
        })
    }
}

enum Source {
    Beacon,
    Dial,
}

type Outbox = mpsc::UnboundedSender<Vec<u8>>;

pub(crate) struct Lan {
    session: u64,
    port: OnceLock<u16>,
    peers: Mutex<HashMap<u64, Outbox>>,
    dialing: Mutex<HashSet<u64>>,
    patience: Mutex<HashMap<u64, u32>>,
    problems: Mutex<Problems>,
}

impl Lan {
    pub(crate) fn new() -> Self {
        Self {
            session: OsRng.next_u64(),
            port: OnceLock::new(),
            peers: Mutex::default(),
            dialing: Mutex::default(),
            patience: Mutex::default(),
            problems: Mutex::default(),
        }
    }

    pub(crate) fn problem(&self) -> Option<LanProblem> {
        lock(&self.problems).current()
    }

    pub(crate) fn port(&self) -> Option<u16> {
        self.port.get().copied()
    }

    pub(crate) fn peer_count(&self) -> usize {
        lock(&self.peers).len()
    }

    pub(crate) fn broadcast(&self, frame: &[u8], except: Option<u64>) {
        for (session, outbox) in lock(&self.peers).iter() {
            if Some(*session) != except {
                let _ = outbox.send(frame.to_vec());
            }
        }
    }

    fn register(&self, session: u64, outbox: &Outbox) -> bool {
        let mut peers = lock(&self.peers);
        if peers.contains_key(&session) {
            return false;
        }
        peers.insert(session, outbox.clone());
        lock(&self.patience).remove(&session);
        true
    }

    fn unregister(&self, session: u64, outbox: &Outbox) {
        let mut peers = lock(&self.peers);
        if peers
            .get(&session)
            .is_some_and(|registered| registered.same_channel(outbox))
        {
            peers.remove(&session);
        }
    }

    /// Whether to dial a peer whose beacon just arrived.
    fn should_dial(&self, peer: u64) -> bool {
        if peer == self.session || lock(&self.peers).contains_key(&peer) {
            return false;
        }
        if self.session > peer {
            let mut patience = lock(&self.patience);
            let beacons = patience.entry(peer).or_insert(0);
            *beacons += 1;
            if *beacons < PATIENCE_BEACONS {
                return false;
            }
        }
        lock(&self.dialing).insert(peer)
    }
}

pub(crate) async fn start(shared: &Arc<Shared>, config: LanConfig) -> Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, config.listen_port)).await?;
    let port = listener.local_addr()?.port();
    if shared.lan.port.set(port).is_err() {
        return Err(Error::Invalid("the local network link is already running"));
    }
    tokio::spawn(accept_loop(Arc::downgrade(shared), listener));
    if let Some(group) = config.discovery {
        tokio::spawn(discovery_loop(Arc::downgrade(shared), group, port));
    }
    Ok(port)
}

/// Connects to a node. `expected` is the session id of a node found by
/// discovery: failures to reach those show in the status bar, while failures
/// of connections the user asked for are reported to the user directly.
pub(crate) async fn dial(
    shared: &Arc<Shared>,
    addr: SocketAddr,
    expected: Option<u64>,
) -> Result<()> {
    let connected = match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
        Ok(connected) => connected,
        Err(_) => Err(io::Error::from(io::ErrorKind::TimedOut)),
    };
    match connected {
        Ok(stream) => {
            report(shared, Source::Dial, None);
            tokio::spawn(run_connection(shared.clone(), stream, expected));
            Ok(())
        }
        Err(error) => {
            if expected.is_some() {
                let context = format!("Found {addr} but could not connect");
                report(
                    shared,
                    Source::Dial,
                    Some(LanProblem::from_error(context, &error)),
                );
            }
            Err(error.into())
        }
    }
}

/// Records how a beacon or connection went, and tells apps when the problem
/// to show changes.
fn report(shared: &Shared, source: Source, problem: Option<LanProblem>) {
    let changed = {
        let mut problems = lock(&shared.lan.problems);
        let before = problems.current();
        match source {
            Source::Beacon => problems.beacon = problem,
            Source::Dial => problems.dial = problem.map(|problem| (problem, Instant::now())),
        }
        let after = problems.current();
        (before != after).then_some(after)
    };
    if let Some(problem) = changed {
        shared.emit(Event::LocalNetworkChanged { problem });
    }
}

async fn accept_loop(shared: Weak<Shared>, listener: TcpListener) {
    loop {
        let accepted = listener.accept().await;
        let Some(shared) = shared.upgrade() else {
            return;
        };
        match accepted {
            Ok((stream, _)) => {
                tokio::spawn(run_connection(shared, stream, None));
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

async fn discovery_loop(shared: Weak<Shared>, group: SocketAddrV4, port: u16) {
    enum Step {
        Beacon,
        Heard(usize, SocketAddr),
        Nothing,
    }

    let Some(session) = shared.upgrade().map(|shared| shared.lan.session) else {
        return;
    };
    let mut beacon = BEACON_PREFIX.to_vec();
    beacon.extend_from_slice(&session.to_be_bytes());
    beacon.extend_from_slice(&port.to_be_bytes());
    let report_beacon = |problem: Option<LanProblem>| {
        if let Some(shared) = shared.upgrade() {
            report(&shared, Source::Beacon, problem);
        }
    };

    let mut socket = None;
    let mut failures = 0;
    let mut tick = tokio::time::interval(BEACON_INTERVAL);
    let mut buffer = [0; 64];
    loop {
        if shared.strong_count() == 0 {
            return;
        }
        let Some(current) = &socket else {
            tick.tick().await;
            match multicast_socket(group) {
                Ok(created) => socket = Some(created),
                Err(error) => report_beacon(Some(LanProblem::from_error(
                    "Could not join the discovery group".to_owned(),
                    &error,
                ))),
            }
            continue;
        };

        let step = tokio::select! {
            _ = tick.tick() => Step::Beacon,
            received = current.recv_from(&mut buffer) => match received {
                Ok((len, from)) => Step::Heard(len, from),
                Err(_) => Step::Nothing,
            },
        };
        match step {
            Step::Beacon => match announce(current, group, &beacon).await {
                Ok(()) => {
                    failures = 0;
                    report_beacon(None);
                }
                Err(error) => {
                    report_beacon(Some(LanProblem::from_error(
                        "Could not announce this computer".to_owned(),
                        &error,
                    )));
                    failures += 1;
                    if failures >= MAX_BEACON_FAILURES {
                        failures = 0;
                        socket = None;
                    }
                }
            },
            Step::Heard(len, from) => {
                let Some((peer, peer_port)) = parse_beacon(&buffer[..len]) else {
                    continue;
                };
                let Some(shared) = shared.upgrade() else {
                    return;
                };
                if shared.lan.should_dial(peer) {
                    let addr = SocketAddr::new(from.ip(), peer_port);
                    tokio::spawn(async move {
                        if dial(&shared, addr, Some(peer)).await.is_err() {
                            lock(&shared.lan.dialing).remove(&peer);
                        }
                    });
                }
            }
            Step::Nothing => {}
        }
    }
}

async fn run_connection(shared: Arc<Shared>, stream: TcpStream, dialed: Option<u64>) {
    let _ = stream.set_nodelay(true);
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let (outbox, queue) = mpsc::unbounded_channel();
    let writer = tokio::spawn(write_loop(writer, queue));

    let mut hello = vec![HELLO];
    hello.extend_from_slice(&shared.lan.session.to_be_bytes());
    let _ = outbox.send(hello);
    let handshake = tokio::time::timeout(READ_TIMEOUT, read_frame(&mut reader)).await;
    if let Some(peer) = dialed {
        lock(&shared.lan.dialing).remove(&peer);
    }
    let peer = match handshake {
        Ok(Ok(frame)) => parse_hello(&frame),
        _ => None,
    };
    let Some(peer) = peer.filter(|&peer| shared.lan.register(peer, &outbox)) else {
        writer.abort();
        return;
    };
    shared.emit(Event::PeersChanged {
        count: shared.lan.peer_count(),
    });

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
    shared.lan.unregister(peer, &outbox);
    shared.emit(Event::PeersChanged {
        count: shared.lan.peer_count(),
    });
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

async fn write_loop(mut writer: OwnedWriteHalf, mut queue: mpsc::UnboundedReceiver<Vec<u8>>) {
    while let Some(frame) = queue.recv().await {
        let Ok(len) = u16::try_from(frame.len()) else {
            continue;
        };
        let mut bytes = len.to_be_bytes().to_vec();
        bytes.extend_from_slice(&frame);
        if writer.write_all(&bytes).await.is_err() {
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

/// Sends the beacon on every local network interface, after joining the
/// group on each: a VPN that takes the default route would otherwise take
/// discovery into its tunnel. Joining again is harmless, and catches
/// interfaces that appeared or came back. Succeeds if any send does.
async fn announce(socket: &UdpSocket, group: SocketAddrV4, beacon: &[u8]) -> io::Result<()> {
    let mut interfaces = discovery_interfaces();
    for interface in &interfaces {
        let _ = socket.join_multicast_v4(*group.ip(), *interface);
    }
    if interfaces.is_empty() {
        // The system's choice, as before interfaces were listed.
        interfaces.push(Ipv4Addr::UNSPECIFIED);
    }
    let mut result = Err(io::Error::other("no interface to announce on"));
    for interface in interfaces {
        let sent = match SockRef::from(socket).set_multicast_if_v4(&interface) {
            Ok(()) => socket.send_to(beacon, group).await.map(drop),
            Err(error) => Err(error),
        };
        if result.is_err() {
            result = sent;
        }
    }
    result
}

/// The IPv4 addresses of the interfaces discovery runs on.
fn discovery_interfaces() -> Vec<Ipv4Addr> {
    let interfaces = if_addrs::get_if_addrs().unwrap_or_default();
    lan_addresses(
        interfaces
            .iter()
            .filter_map(|interface| match &interface.addr {
                if_addrs::IfAddr::V4(v4) => {
                    Some((interface.is_loopback(), v4.ip, v4.broadcast.is_some()))
                }
                if_addrs::IfAddr::V6(_) => None,
            }),
    )
}

/// Of `(loopback, address, can broadcast)`, the addresses of interfaces onto
/// a local network: not loopback, and not point-to-point like a VPN tunnel,
/// which has no broadcast address.
fn lan_addresses(interfaces: impl Iterator<Item = (bool, Ipv4Addr, bool)>) -> Vec<Ipv4Addr> {
    let mut addresses: Vec<Ipv4Addr> = interfaces
        .filter(|&(loopback, _, broadcast)| !loopback && broadcast)
        .map(|(_, address, _)| address)
        .collect();
    addresses.sort();
    addresses.dedup();
    addresses
}

fn parse_beacon(bytes: &[u8]) -> Option<(u64, u16)> {
    if bytes.len() != BEACON_LEN || !bytes.starts_with(BEACON_PREFIX) {
        return None;
    }
    let session = u64::from_be_bytes(bytes[6..14].try_into().ok()?);
    let port = u16::from_be_bytes(bytes[14..16].try_into().ok()?);
    Some((session, port))
}

fn multicast_socket(group: SocketAddrV4) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    // Lets several nodes on one computer share the discovery port.
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, group.port()).into())?;
    socket.join_multicast_v4(group.ip(), &Ipv4Addr::UNSPECIFIED)?;
    socket.set_multicast_loop_v4(true)?;
    socket.set_nonblocking(true)?;
    UdpSocket::from_std(socket.into())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_skips_loopback_and_tunnels() {
        let wifi = Ipv4Addr::new(192, 168, 1, 18);
        let cable = Ipv4Addr::new(169, 254, 10, 2);
        let interfaces = [
            (true, Ipv4Addr::LOCALHOST, false),
            (false, wifi, true),
            (false, Ipv4Addr::new(10, 8, 0, 3), false),
            (false, cable, true),
            (false, wifi, true),
        ];
        assert_eq!(lan_addresses(interfaces.into_iter()), vec![cable, wifi]);
        assert!(lan_addresses(std::iter::empty()).is_empty());
    }

    fn os_error(code: i32) -> io::Error {
        io::Error::from_raw_os_error(code)
    }

    #[test]
    fn macos_refusals_read_as_blocked() {
        // EHOSTUNREACH and EPERM, as macOS returns them without the permission.
        let unreachable = LanProblem::from_error("Could not announce".to_owned(), &os_error(65));
        let refused = LanProblem::from_error(
            "Found 10.0.0.2:1 but could not connect".to_owned(),
            &os_error(1),
        );
        let other = LanProblem::from_error(
            "x".to_owned(),
            &io::Error::from(io::ErrorKind::ConnectionRefused),
        );
        if cfg!(target_os = "macos") {
            assert!(matches!(unreachable, LanProblem::Blocked { .. }));
            assert!(matches!(refused, LanProblem::Blocked { .. }));
        }
        assert!(matches!(other, LanProblem::Failed { .. }));
    }

    #[test]
    fn beacon_problems_win_and_dial_problems_fade() {
        let blocked = LanProblem::Blocked {
            detail: "beacon".to_owned(),
        };
        let failed = LanProblem::Failed {
            detail: "dial".to_owned(),
        };
        let mut problems = Problems {
            beacon: None,
            dial: Some((failed.clone(), Instant::now())),
        };
        assert_eq!(problems.current(), Some(failed.clone()));
        problems.beacon = Some(blocked.clone());
        assert_eq!(problems.current(), Some(blocked));
        problems.beacon = None;
        problems.dial = Some((failed, Instant::now() - DIAL_PROBLEM_TTL));
        assert_eq!(problems.current(), None);
    }
}

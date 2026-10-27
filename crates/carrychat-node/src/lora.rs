//! The LoRa link (§9.2): frames travel as Meshtastic packets, broadcast on
//! the device's primary channel. The node talks to the device over TCP (a
//! device on the network, or meshtasticd) or a USB serial port, or platform
//! code connects it, as Android does over Bluetooth LE.

pub mod meshtastic;

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use carrychat_proto::frame::{frames_for, Frame};
use prost::Message;
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, Notify};
use tokio::task::AbortHandle;

use self::meshtastic::{
    from_radio, mesh_packet, to_radio, Data, Deframer, FromRadio, Heartbeat, MeshPacket,
    Modulation, NodeInfo, ToRadio, User,
};
use crate::{now, Error, Event, Result, Shared};

/// Share of the time this node keeps the radio busy at most, and the burst
/// it may spend at once. The legal limit at 869.4–869.65 MHz is 10%, shared
/// by every device in range.
const AIRTIME_SHARE: f64 = 0.025;
const AIRTIME_BURST_SECS: f64 = 20.0;
/// Frames waiting for airtime. Past these, the oldest carried one goes first,
/// then the oldest own one.
const MAX_CARRIED: usize = 32;
const MAX_OWN: usize = 256;
const RECONNECT_DELAY: Duration = Duration::from_secs(10);
const CONFIG_TIMEOUT: Duration = Duration::from_secs(30);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(300);
const DEFAULT_HOP_LIMIT: u32 = 3;
/// How far back the devices heard count: Meshtastic devices announce
/// themselves every three hours by default.
const HEARD_WITHIN_SECS: u32 = 3 * 60 * 60;

/// Where the Meshtastic device is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoraDevice {
    /// `host:port` of a device on the network, or of meshtasticd, which
    /// listens on port 4403.
    Tcp(String),
    /// A USB serial port, such as `/dev/cu.usbserial-0001`.
    Serial(PathBuf),
    /// A device's Bluetooth LE address, which platform code connects to and
    /// hands over with `Node::lora_over`.
    Bluetooth(String),
}

impl fmt::Display for LoraDevice {
    /// As `from_str` reads it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp(address) => write!(f, "tcp:{address}"),
            Self::Serial(path) => write!(f, "{}", path.display()),
            Self::Bluetooth(address) => write!(f, "bluetooth:{address}"),
        }
    }
}

impl FromStr for LoraDevice {
    type Err = Error;

    /// `tcp:host:port`, `bluetooth:address` or a serial port's path.
    fn from_str(text: &str) -> Result<Self> {
        if let Some(address) = text.strip_prefix("bluetooth:") {
            return match address {
                "" => Err(Error::Invalid("give the device as bluetooth:address")),
                address => Ok(Self::Bluetooth(address.to_owned())),
            };
        }
        match text.strip_prefix("tcp:") {
            Some(address) if !address.is_empty() => Ok(Self::Tcp(address.to_owned())),
            Some(_) => Err(Error::Invalid("give the device as tcp:host:port")),
            None if text.is_empty() => Err(Error::Invalid("give a device")),
            None => Ok(Self::Serial(PathBuf::from(text))),
        }
    }
}

/// How the LoRa link stands, for the app.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoraStatus {
    /// The device in use, as `LoraDevice` writes it.
    pub device: Option<String>,
    pub connected: bool,
    /// The device's own name.
    pub name: Option<String>,
    /// The radio region it is set to, such as `EU_868`. A device whose
    /// region is `UNSET` does not transmit.
    pub region: Option<String>,
    /// Frames waiting for airtime.
    pub waiting: usize,
    /// Other Meshtastic devices heard in the last three hours, latest first.
    pub heard: Vec<LoraNode>,
    /// When CarryChat last came in over LoRa, in seconds since the epoch.
    pub carrychat_at: Option<u32>,
}

/// A Meshtastic device the device heard.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoraNode {
    pub name: String,
    /// When it was last heard, in seconds since the epoch.
    pub at: u32,
    /// The signal-to-noise ratio it was last heard with, in dB.
    pub snr: Option<f32>,
    /// Hops between it and the device; 0 when heard directly.
    pub hops: Option<u32>,
}

pub(crate) struct Lora {
    started: AtomicBool,
    connected: AtomicBool,
    queue: Mutex<Queue>,
    wake: Arc<Notify>,
    device: Mutex<Option<LoraDevice>>,
    /// The loop that connects to a TCP or serial device.
    task: Mutex<Option<AbortHandle>>,
    seen: Mutex<Seen>,
}

/// What the device told about itself and the devices it hears.
#[derive(Default)]
struct Seen {
    me: Option<u32>,
    region: Option<i32>,
    nodes: HashMap<u32, Heard>,
    carrychat_at: Option<u32>,
}

#[derive(Default)]
struct Heard {
    name: Option<String>,
    at: Option<u32>,
    snr: Option<f32>,
    hops: Option<u32>,
}

impl Seen {
    fn node_info(&mut self, info: NodeInfo, now: u32) {
        let heard = self.nodes.entry(info.num).or_default();
        if let Some(user) = &info.user {
            heard.name = Some(display_name(user));
        }
        // The device's clock may never have been set.
        if info.last_heard > 1_600_000_000 && info.last_heard <= now + 86_400 {
            heard.at = heard.at.max(Some(info.last_heard));
        }
        if info.snr != 0.0 {
            heard.snr = Some(info.snr);
        }
        if info.hops_away.is_some() {
            heard.hops = info.hops_away;
        }
    }

    fn packet(&mut self, packet: &MeshPacket, now: u32) {
        if packet.from == 0 || Some(packet.from) == self.me {
            return;
        }
        let heard = self.nodes.entry(packet.from).or_default();
        heard.at = Some(now);
        if packet.rx_snr != 0.0 {
            heard.snr = Some(packet.rx_snr);
        }
        if packet.hop_start > 0 && packet.hop_start >= packet.hop_limit {
            heard.hops = Some(packet.hop_start - packet.hop_limit);
        }
        if let Some(mesh_packet::Payload::Decoded(data)) = &packet.payload {
            if data.portnum == meshtastic::NODEINFO_APP {
                if let Ok(user) = User::decode(data.payload.as_slice()) {
                    heard.name = Some(display_name(&user));
                }
            }
        }
    }
}

/// A device's long name, or else its short name or id.
fn display_name(user: &User) -> String {
    [&user.long_name, &user.short_name, &user.id]
        .into_iter()
        .find(|name| !name.is_empty())
        .cloned()
        .unwrap_or_default()
}

#[derive(Default)]
struct Queue {
    own: VecDeque<Vec<u8>>,
    carried: VecDeque<Vec<u8>>,
}

impl Queue {
    fn len(&self) -> usize {
        self.own.len() + self.carried.len()
    }

    fn peek_len(&self) -> Option<usize> {
        self.own.front().or(self.carried.front()).map(Vec::len)
    }

    fn next(&mut self) -> Option<Vec<u8>> {
        self.own.pop_front().or_else(|| self.carried.pop_front())
    }
}

impl Lora {
    pub(crate) fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            connected: AtomicBool::new(false),
            queue: Mutex::default(),
            wake: Arc::new(Notify::new()),
            device: Mutex::new(None),
            task: Mutex::new(None),
            seen: Mutex::default(),
        }
    }

    pub(crate) fn status(&self, now: u32) -> LoraStatus {
        let seen = lock(&self.seen);
        let mut heard: Vec<LoraNode> = seen
            .nodes
            .iter()
            .filter(|(num, _)| Some(**num) != seen.me)
            .filter_map(|(num, heard)| {
                let at = heard
                    .at
                    .filter(|at| now.saturating_sub(*at) <= HEARD_WITHIN_SECS)?;
                Some(LoraNode {
                    name: heard.name.clone().unwrap_or_else(|| format!("!{num:08x}")),
                    at,
                    snr: heard.snr,
                    hops: heard.hops,
                })
            })
            .collect();
        heard.sort_by(|a, b| b.at.cmp(&a.at));
        LoraStatus {
            device: lock(&self.device).as_ref().map(ToString::to_string),
            connected: self.connected(),
            name: seen
                .me
                .and_then(|me| seen.nodes.get(&me))
                .and_then(|me| me.name.clone()),
            region: seen.region.map(meshtastic::region_name),
            waiting: self.waiting(),
            heard,
            carrychat_at: seen.carrychat_at,
        }
    }

    /// The device in use, if any.
    pub(crate) fn device(&self) -> Option<LoraDevice> {
        lock(&self.device).clone()
    }

    pub(crate) fn connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub(crate) fn waiting(&self) -> usize {
        lock(&self.queue).len()
    }

    /// Queues a frame for the radio. Only bundles and tombstones go over
    /// LoRa: never file chunks. `own` frames, the node's bundles and
    /// tombstones, go before ones it carries for others. Returns whether it
    /// was queued for a connected device.
    pub(crate) fn send(&self, frame: &Frame, own: bool) -> bool {
        if !self.started.load(Ordering::Relaxed) {
            return false;
        }
        let packets = match frame {
            Frame::Bundle(bundle) => match frames_for(bundle, meshtastic::MTU) {
                Ok(frames) => frames.iter().map(Frame::to_bytes).collect(),
                Err(_) => return false,
            },
            Frame::Tombstone { .. } => vec![frame.to_bytes()],
            Frame::Fragment(_) | Frame::Chunk(_) => return false,
        };
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
        self.connected()
    }
}

/// Uses `device` for the link from now on, in place of any other. The node
/// connects to TCP and serial devices itself, and again whenever the
/// connection ends; platform code connects Bluetooth ones.
pub(crate) fn start(shared: &Arc<Shared>, device: LoraDevice) -> Result<()> {
    stop(shared);
    let lora = &shared.lora;
    *lock(&lora.device) = Some(device.clone());
    lora.started.store(true, Ordering::Relaxed);
    if !matches!(device, LoraDevice::Bluetooth(_)) {
        let task = tokio::spawn(run(Arc::downgrade(shared), device));
        *lock(&lora.task) = Some(task.abort_handle());
    }
    Ok(())
}

/// Stops using the device, and forgets what it said.
pub(crate) fn stop(shared: &Shared) {
    let lora = &shared.lora;
    if let Some(task) = lock(&lora.task).take() {
        task.abort();
    }
    lora.started.store(false, Ordering::Relaxed);
    *lock(&lora.device) = None;
    *lock(&lora.queue) = Queue::default();
    *lock(&lora.seen) = Seen::default();
    if lora.connected.swap(false, Ordering::Relaxed) {
        shared.emit(Event::LoraChanged { connected: false });
    }
}

/// Connects to the device, and again whenever the connection ends.
async fn run(shared: Weak<Shared>, device: LoraDevice) {
    loop {
        let result = match open(&device).await {
            Ok(stream) => session(&shared, stream).await,
            Err(error) => Err(error),
        };
        if !ended(&shared, &result) {
            return;
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// Runs the link over a connection platform code made to the device,
/// carrying the stream framing as a serial port does, until it ends.
pub(crate) async fn over<R, W>(shared: &Arc<Shared>, reader: R, writer: W) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    if !shared.lora.started.load(Ordering::Relaxed) {
        return Err(Error::Invalid("no LoRa device is chosen"));
    }
    let shared = Arc::downgrade(shared);
    let result = session(&shared, stream_of(reader, writer)).await;
    ended(&shared, &result);
    Ok(result?)
}

/// Notes that a connection to the device ended. Returns whether the node is
/// still there.
fn ended(shared: &Weak<Shared>, result: &io::Result<()>) -> bool {
    let Some(node) = shared.upgrade() else {
        return false;
    };
    if node.lora.connected.swap(false, Ordering::Relaxed) {
        node.emit(Event::LoraChanged { connected: false });
    }
    if let Err(error) = result {
        eprintln!("LoRa: {error}");
    }
    true
}

/// The bytes to and from a device, whatever carries them.
struct Stream {
    incoming: mpsc::UnboundedReceiver<Vec<u8>>,
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
}

async fn open(device: &LoraDevice) -> io::Result<Stream> {
    match device {
        LoraDevice::Tcp(address) => {
            let stream = tokio::net::TcpStream::connect(address).await?;
            let _ = stream.set_nodelay(true);
            let (reader, writer) = stream.into_split();
            Ok(stream_of(reader, writer))
        }
        LoraDevice::Serial(path) => open_serial(path),
        LoraDevice::Bluetooth(_) => {
            Err(io::Error::other("platform code connects Bluetooth devices"))
        }
    }
}

fn stream_of<R, W>(mut reader: R, mut writer: W) -> Stream
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (incoming_tx, incoming) = mpsc::unbounded_channel();
    let (outgoing, mut outgoing_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        let mut buffer = [0; 1024];
        while let Ok(len @ 1..) = reader.read(&mut buffer).await {
            if incoming_tx.send(buffer[..len].to_vec()).is_err() {
                return;
            }
        }
    });
    tokio::spawn(async move {
        while let Some(bytes) = outgoing_rx.recv().await {
            if writer.write_all(&bytes).await.is_err() {
                return;
            }
        }
    });
    Stream { incoming, outgoing }
}

/// A serial port a Meshtastic device may be on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoraPort {
    pub path: String,
    /// What the USB device calls itself, if it says.
    pub label: Option<String>,
}

/// The USB serial ports on this computer. On macOS each port shows up twice;
/// only the `cu.` one, which does not wait for a carrier, is listed.
#[cfg(not(target_os = "android"))]
pub fn ports() -> Vec<LoraPort> {
    let Ok(ports) = serialport::available_ports() else {
        return Vec::new();
    };
    ports
        .into_iter()
        .filter(|port| !port.port_name.starts_with("/dev/tty."))
        .filter_map(|port| match port.port_type {
            serialport::SerialPortType::UsbPort(usb) => Some(LoraPort {
                path: port.port_name,
                label: usb.product.or(usb.manufacturer),
            }),
            _ => None,
        })
        .collect()
}

#[cfg(not(target_os = "android"))]
fn open_serial(path: &std::path::Path) -> io::Result<Stream> {
    use std::io::{Read, Write};

    let port = serialport::new(path.to_string_lossy(), 115_200)
        .timeout(Duration::from_millis(500))
        .open()
        .map_err(io::Error::other)?;
    let mut reader = port.try_clone().map_err(io::Error::other)?;
    let mut writer = port;
    let (incoming_tx, incoming) = mpsc::unbounded_channel();
    let (outgoing, mut outgoing_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buffer = [0; 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => return,
                Ok(len) => {
                    if incoming_tx.send(buffer[..len].to_vec()).is_err() {
                        return;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                    if incoming_tx.is_closed() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    });
    std::thread::spawn(move || {
        while let Some(bytes) = outgoing_rx.blocking_recv() {
            if writer
                .write_all(&bytes)
                .and_then(|()| writer.flush())
                .is_err()
            {
                return;
            }
        }
    });
    // A device whose serial port is printing its log switches to the API
    // when it reads a run of START2 bytes.
    let _ = outgoing.send(vec![meshtastic::START2; 32]);
    Ok(Stream { incoming, outgoing })
}

#[cfg(target_os = "android")]
fn open_serial(_path: &std::path::Path) -> io::Result<Stream> {
    Err(io::Error::other(
        "serial ports are not supported on Android",
    ))
}

/// What the device reported about its radio.
struct Radio {
    modulation: Modulation,
    hop_limit: u32,
}

/// One connection: asks for the configuration, then sends from the queue
/// within the airtime budget and takes in the packets the device hears.
async fn session(shared: &Weak<Shared>, mut stream: Stream) -> io::Result<()> {
    let send = |message: &ToRadio| {
        stream
            .outgoing
            .send(meshtastic::frame(message))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
    };
    let config_id = OsRng.next_u32().max(1);
    send(&ToRadio {
        payload: Some(to_radio::Payload::WantConfigId(config_id)),
    })?;

    let mut deframer = Deframer::default();
    let mut radio = Radio {
        modulation: Modulation::LONG_FAST,
        hop_limit: DEFAULT_HOP_LIMIT,
    };
    // The configuration first; packets heard meanwhile are taken in.
    let configured = tokio::time::timeout(CONFIG_TIMEOUT, async {
        loop {
            let bytes = stream
                .incoming
                .recv()
                .await
                .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
            deframer.push(&bytes);
            while let Some(body) = deframer.next_frame() {
                let Ok(message) = FromRadio::decode(body.as_slice()) else {
                    continue;
                };
                match message.payload {
                    Some(from_radio::Payload::Config(config)) => {
                        if let Some(meshtastic::config::Payload::Lora(lora)) = config.payload {
                            radio.modulation = Modulation::of(&lora);
                            if lora.hop_limit > 0 {
                                radio.hop_limit = lora.hop_limit;
                            }
                            if let Some(node) = shared.upgrade() {
                                lock(&node.lora.seen).region = Some(lora.region);
                            }
                        }
                    }
                    Some(from_radio::Payload::ConfigCompleteId(id)) if id == config_id => {
                        return Ok::<(), io::Error>(());
                    }
                    Some(payload) => note(shared, payload),
                    None => {}
                }
            }
        }
    })
    .await;
    match configured {
        Ok(result) => result?,
        Err(_) => return Err(io::Error::other("the device sent no configuration")),
    }

    if let Some(node) = shared.upgrade() {
        node.lora.connected.store(true, Ordering::Relaxed);
        node.emit(Event::LoraChanged { connected: true });
    }
    let Some(wake) = shared.upgrade().map(|node| Arc::clone(&node.lora.wake)) else {
        return Ok(());
    };
    let mut budget = Budget::new();
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.tick().await;
    loop {
        let Some(node) = shared.upgrade() else {
            return Ok(());
        };
        let airtime = |len: usize| radio.modulation.airtime(len + meshtastic::OVERHEAD);
        let next_len = lock(&node.lora.queue).peek_len();
        let wait = next_len.and_then(|len| budget.wait_for(airtime(len)));
        let ready = next_len.is_some() && wait.is_none();
        let frame = if ready {
            lock(&node.lora.queue).next()
        } else {
            None
        };
        drop(node);
        if let Some(frame) = frame {
            budget.spend(airtime(frame.len()));
            send(&ToRadio {
                payload: Some(to_radio::Payload::Packet(MeshPacket {
                    to: meshtastic::BROADCAST,
                    id: OsRng.next_u32().max(1),
                    hop_limit: radio.hop_limit,
                    payload: Some(mesh_packet::Payload::Decoded(Data {
                        portnum: meshtastic::PRIVATE_APP,
                        payload: frame,
                    })),
                    ..MeshPacket::default()
                })),
            })?;
            continue;
        }
        tokio::select! {
            bytes = stream.incoming.recv() => {
                let bytes = bytes.ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
                deframer.push(&bytes);
                while let Some(body) = deframer.next_frame() {
                    if let Ok(FromRadio { payload: Some(payload), .. }) = FromRadio::decode(body.as_slice()) {
                        note(shared, payload);
                    }
                }
            }
            () = wake.notified() => {}
            () = tokio::time::sleep(wait.unwrap_or_default()), if wait.is_some() => {}
            _ = heartbeat.tick() => {
                send(&ToRadio { payload: Some(to_radio::Payload::Heartbeat(Heartbeat {})) })?;
            }
        }
    }
}

/// Takes in what the device says: itself, the devices it knows, and the
/// packets it hears.
fn note(shared: &Weak<Shared>, payload: from_radio::Payload) {
    let Some(node) = shared.upgrade() else {
        return;
    };
    match payload {
        from_radio::Payload::MyInfo(info) => lock(&node.lora.seen).me = Some(info.my_node_num),
        from_radio::Payload::NodeInfo(info) => lock(&node.lora.seen).node_info(info, now()),
        from_radio::Payload::Packet(packet) => {
            lock(&node.lora.seen).packet(&packet, now());
            drop(node);
            take(shared, packet);
        }
        _ => {}
    }
}

/// Takes in a CarryChat frame the device heard. It is handled like a frame
/// from any link, and passed on to every link but LoRa: the mesh already
/// floods it there.
fn take(shared: &Weak<Shared>, packet: MeshPacket) {
    let Some(mesh_packet::Payload::Decoded(data)) = packet.payload else {
        return;
    };
    if data.portnum != meshtastic::PRIVATE_APP {
        return;
    }
    let Some(shared) = shared.upgrade() else {
        return;
    };
    // A frame that does not parse is skipped, as on every link; file chunks
    // never belong on LoRa.
    let frame = match Frame::from_bytes(&data.payload) {
        Ok(Frame::Chunk(_)) | Err(_) => return,
        Ok(frame) => frame,
    };
    lock(&shared.lora.seen).carrychat_at = Some(now());
    if let Ok(handled) = shared.handle(frame.clone()) {
        shared.dispatch_from_lora(handled, &frame);
    }
}

/// Whether `frame`, to be passed on, is the one `heard` over LoRa: the same
/// bundle, whole or as its fragment, or the same tombstone. A receipt for a
/// bundle heard is a new frame, not an echo.
pub(crate) fn echoes(frame: &Frame, heard: &Frame) -> bool {
    match (frame, heard) {
        (Frame::Bundle(bundle), Frame::Bundle(heard)) => bundle.id == heard.id,
        (Frame::Bundle(bundle), Frame::Fragment(heard)) => bundle.id == heard.id,
        (
            Frame::Tombstone { tombstone, .. },
            Frame::Tombstone {
                tombstone: heard, ..
            },
        ) => tombstone.id == heard.id,
        _ => false,
    }
}

/// Airtime this node may still spend: it refills at `AIRTIME_SHARE` of the
/// time and holds at most `AIRTIME_BURST_SECS`.
struct Budget {
    seconds: f64,
    at: Instant,
}

impl Budget {
    fn new() -> Self {
        Self {
            seconds: AIRTIME_BURST_SECS,
            at: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.at).as_secs_f64();
        self.seconds = (self.seconds + elapsed * AIRTIME_SHARE).min(AIRTIME_BURST_SECS);
        self.at = now;
    }

    /// How long until `airtime` seconds may be spent, or `None` if now.
    fn wait_for(&mut self, airtime: f64) -> Option<Duration> {
        self.refill();
        (self.seconds < airtime)
            .then(|| Duration::from_secs_f64((airtime - self.seconds) / AIRTIME_SHARE))
    }

    fn spend(&mut self, airtime: f64) {
        self.refill();
        self.seconds -= airtime;
    }
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
    fn devices_are_named_by_address_or_path() {
        assert_eq!(
            "tcp:127.0.0.1:4403".parse::<LoraDevice>().unwrap(),
            LoraDevice::Tcp("127.0.0.1:4403".to_owned())
        );
        assert_eq!(
            "/dev/cu.usbserial-0001".parse::<LoraDevice>().unwrap(),
            LoraDevice::Serial(PathBuf::from("/dev/cu.usbserial-0001"))
        );
        assert!("tcp:".parse::<LoraDevice>().is_err());
        assert_eq!(
            "bluetooth:AA:BB:CC:DD:EE:FF".parse::<LoraDevice>().unwrap(),
            LoraDevice::Bluetooth("AA:BB:CC:DD:EE:FF".to_owned())
        );
        for text in [
            "tcp:127.0.0.1:4403",
            "/dev/cu.usbserial-0001",
            "bluetooth:AA:BB:CC:DD:EE:FF",
        ] {
            assert_eq!(text.parse::<LoraDevice>().unwrap().to_string(), text);
        }
    }

    #[test]
    fn the_budget_allows_a_burst_then_paces() {
        let mut budget = Budget::new();
        let packet = Modulation::LONG_FAST.airtime(meshtastic::MTU + meshtastic::OVERHEAD);
        let mut sent = 0;
        while budget.wait_for(packet).is_none() {
            budget.spend(packet);
            sent += 1;
        }
        // About nine full packets fit the burst; the next waits ~a minute and a half.
        assert!((8..=10).contains(&sent), "{sent}");
        let wait = budget.wait_for(packet).unwrap();
        assert!(
            wait > Duration::from_secs(30) && wait < Duration::from_secs(120),
            "{wait:?}"
        );
    }
}

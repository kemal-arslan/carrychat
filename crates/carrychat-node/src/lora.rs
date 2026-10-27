//! The LoRa link (§9.2): frames travel through a Meshtastic device, as
//! packets broadcast on its primary channel, or a MeshCore device, as
//! datagrams flooded on a channel of their own. The node talks to the device
//! over TCP (a device on the network, or meshtasticd) or a USB serial port,
//! and finds out which kind it is, or platform code connects it, as Android
//! does over Bluetooth LE.

pub mod meshcore;
pub mod meshtastic;

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use carrychat_proto::bundle::BundleId;
use carrychat_proto::frame::{frames_for, Frame};
use prost::Message;
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, Notify};
use tokio::task::AbortHandle;

use self::meshcore::{ChannelData, Contact, SelfInfo};
use self::meshtastic::{
    from_radio, mesh_packet, to_radio, Data, Deframer, FromRadio, Heartbeat, MeshPacket,
    Modulation, NodeInfo, ToRadio, User,
};
use crate::{now, Error, Event, Result, Shared, Via};

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
/// Receipts for the latest messages delivered to us that go out again when
/// a device connects: airtime is too dear for more.
const PENDING_RECEIPTS: usize = 8;
const DEFAULT_HOP_LIMIT: u32 = 3;
/// How long a device has to answer as MeshCore does before it is taken for
/// a Meshtastic one. The question is asked twice, for a device that resets
/// as its serial port opens.
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
/// How long a MeshCore device has to answer a command.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the node asks a MeshCore device for messages it did not say
/// were waiting, and for the devices it knows.
const SYNC_INTERVAL: Duration = Duration::from_secs(60);
const CONTACTS_INTERVAL: Duration = Duration::from_secs(600);
/// How long to wait when a MeshCore device's send queue is full.
const BUSY_DELAY: Duration = Duration::from_secs(2);

/// What a LoRa device runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum LoraFirmware {
    Meshtastic,
    MeshCore,
}

impl LoraFirmware {
    /// The most a packet carries.
    fn mtu(self) -> usize {
        match self {
            Self::Meshtastic => meshtastic::MTU,
            Self::MeshCore => meshcore::MTU,
        }
    }

    /// How far back the devices heard count: Meshtastic devices announce
    /// themselves every three hours by default, MeshCore repeaters every 47.
    fn heard_within(self) -> u32 {
        match self {
            Self::Meshtastic => 3 * 60 * 60,
            Self::MeshCore => 48 * 60 * 60,
        }
    }
}

/// Where the Meshtastic or MeshCore device is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoraDevice {
    /// `host:port` of a device on the network: Meshtastic devices and
    /// meshtasticd listen on port 4403, MeshCore devices on 5000.
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
    /// What the device runs, once it has answered.
    pub firmware: Option<LoraFirmware>,
    /// The device's own name.
    pub name: Option<String>,
    /// The radio region a Meshtastic device is set to, such as `EU_868`.
    /// A device whose region is `UNSET` does not transmit.
    pub region: Option<String>,
    /// A MeshCore device's frequency, spreading factor and bandwidth.
    pub radio: Option<String>,
    /// A MeshCore device with every channel slot taken, so CarryChat's
    /// channel could not be added.
    pub no_free_channel: bool,
    /// Frames waiting for airtime.
    pub waiting: usize,
    /// Other devices heard in the last `heard_within` seconds, latest first.
    pub heard: Vec<LoraNode>,
    pub heard_within: u32,
    /// When CarryChat last came in over LoRa, in seconds since the epoch.
    pub carrychat_at: Option<u32>,
}

/// A device the device heard.
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
    /// The most a packet carries: a MeshCore one's, until the device says
    /// it runs Meshtastic.
    mtu: AtomicUsize,
    queue: Mutex<Queue>,
    wake: Arc<Notify>,
    device: Mutex<Option<LoraDevice>>,
    /// The loop that connects to a TCP or serial device.
    task: Mutex<Option<AbortHandle>>,
    seen: Mutex<Seen>,
    /// The bundles and tombstones queued since the link started, so that
    /// connecting again does not send them twice.
    queued: Mutex<HashSet<BundleId>>,
}

/// What the device told about itself and the devices it hears, by their
/// Meshtastic node number or the start of their MeshCore public key.
#[derive(Default)]
struct Seen {
    firmware: Option<LoraFirmware>,
    me: Option<u32>,
    region: Option<i32>,
    radio: Option<String>,
    no_free_channel: bool,
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
        if plausible(info.last_heard, now) {
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

    fn contact(&mut self, contact: &Contact, now: u32) {
        let heard = self
            .nodes
            .entry(meshcore::key_number(&contact.public_key))
            .or_default();
        if !contact.name.is_empty() {
            heard.name = Some(contact.name.clone());
        }
        if plausible(contact.heard_at, now) {
            heard.at = heard.at.max(Some(contact.heard_at));
        }
        heard.hops = contact.hops();
    }

    fn advert(&mut self, key: u32, now: u32) {
        self.nodes.entry(key).or_default().at = Some(now);
    }
}

/// Whether a time from the device's clock, which may never have been set,
/// can be right.
fn plausible(at: u32, now: u32) -> bool {
    at > 1_600_000_000 && at <= now + 86_400
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

    /// Puts back a packet the device could not take yet, to go first.
    fn put_back(&mut self, packet: Vec<u8>) {
        self.own.push_front(packet);
    }
}

impl Lora {
    pub(crate) fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            connected: AtomicBool::new(false),
            mtu: AtomicUsize::new(meshcore::MTU),
            queue: Mutex::default(),
            wake: Arc::new(Notify::new()),
            device: Mutex::new(None),
            task: Mutex::new(None),
            seen: Mutex::default(),
            queued: Mutex::default(),
        }
    }

    pub(crate) fn status(&self, now: u32) -> LoraStatus {
        let seen = lock(&self.seen);
        let heard_within = seen
            .firmware
            .unwrap_or(LoraFirmware::Meshtastic)
            .heard_within();
        let mut heard: Vec<LoraNode> = seen
            .nodes
            .iter()
            .filter(|(num, _)| Some(**num) != seen.me)
            .filter_map(|(num, heard)| {
                let at = heard
                    .at
                    .filter(|at| now.saturating_sub(*at) <= heard_within)?;
                Some(LoraNode {
                    name: heard.name.clone().unwrap_or_else(|| format!("!{num:08x}")),
                    at,
                    snr: heard.snr,
                    hops: heard.hops,
                })
            })
            .collect();
        heard.sort_by_key(|node| std::cmp::Reverse(node.at));
        LoraStatus {
            device: lock(&self.device).as_ref().map(ToString::to_string),
            connected: self.connected(),
            firmware: seen.firmware,
            name: seen
                .me
                .and_then(|me| seen.nodes.get(&me))
                .and_then(|me| me.name.clone()),
            region: seen.region.map(meshtastic::region_name),
            radio: seen.radio.clone(),
            no_free_channel: seen.no_free_channel,
            waiting: self.waiting(),
            heard,
            heard_within,
            carrychat_at: seen.carrychat_at,
        }
    }

    /// Queues one of our own frames that waited for the link, unless it was
    /// queued since the link started. Returns whether it was queued.
    fn send_pending(&self, frame: &Frame) -> bool {
        let queued = queued_id(frame).is_some_and(|id| lock(&self.queued).contains(&id));
        !queued && self.send(frame, true)
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
            Frame::Bundle(bundle) => match frames_for(bundle, self.mtu.load(Ordering::Relaxed)) {
                Ok(frames) => frames.iter().map(Frame::to_bytes).collect(),
                Err(_) => return false,
            },
            Frame::Tombstone { .. } => vec![frame.to_bytes()],
            Frame::Fragment(_) | Frame::Chunk(_) => return false,
        };
        if let Some(id) = queued_id(frame) {
            lock(&self.queued).insert(id);
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
    lock(&lora.queued).clear();
    lora.mtu.store(meshcore::MTU, Ordering::Relaxed);
    if lora.connected.swap(false, Ordering::Relaxed) {
        shared.emit(Event::LoraChanged { connected: false });
    }
}

/// Connects to the device, and again whenever the connection ends.
async fn run(shared: Weak<Shared>, device: LoraDevice) {
    loop {
        let result = match open(&device).await {
            Ok(stream) => session(&shared, stream, None).await,
            Err(error) => Err(error),
        };
        if !ended(&shared, &result) {
            return;
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// Runs the link over a connection platform code made to a device running
/// `firmware`, carrying the stream framing as a serial port does, until it
/// ends.
pub(crate) async fn over<R, W>(
    shared: &Arc<Shared>,
    firmware: LoraFirmware,
    reader: R,
    writer: W,
) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    if !shared.lora.started.load(Ordering::Relaxed) {
        return Err(Error::Invalid("no LoRa device is chosen"));
    }
    let shared = Arc::downgrade(shared);
    let result = session(&shared, stream_of(reader, writer), Some(firmware)).await;
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

/// A serial port a LoRa device may be on.
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

/// One connection: finds out what the device runs, unless platform code
/// knows, then speaks its protocol until the connection ends.
async fn session(
    shared: &Weak<Shared>,
    mut stream: Stream,
    firmware: Option<LoraFirmware>,
) -> io::Result<()> {
    let firmware = match firmware {
        Some(firmware) => firmware,
        None => probe(&mut stream).await?,
    };
    if let Some(node) = shared.upgrade() {
        node.lora.mtu.store(firmware.mtu(), Ordering::Relaxed);
        lock(&node.lora.seen).firmware = Some(firmware);
    }
    match firmware {
        LoraFirmware::Meshtastic => meshtastic_session(shared, stream).await,
        LoraFirmware::MeshCore => meshcore_session(shared, stream).await,
    }
}

/// Asks the device what it is as MeshCore's apps do; one that does not
/// answer is taken for a Meshtastic device. MeshCore goes first: over TCP,
/// a MeshCore device loses track of its frames when Meshtastic's come in,
/// while a Meshtastic device skips what is not its own.
async fn probe(stream: &mut Stream) -> io::Result<LoraFirmware> {
    let ask = |stream: &Stream| {
        stream
            .outgoing
            .send(meshcore::frame(&meshcore::app_start()))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
    };
    ask(stream)?;
    let mut deframer = meshcore::Deframer::new(meshcore::FROM_DEVICE);
    let mut again = Box::pin(tokio::time::sleep(PROBE_TIMEOUT / 2));
    let mut asked_again = false;
    let answered = tokio::time::timeout(PROBE_TIMEOUT, async {
        loop {
            tokio::select! {
                bytes = stream.incoming.recv() => {
                    let bytes = bytes.ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
                    deframer.push(&bytes);
                    while let Some(body) = deframer.next_frame() {
                        if SelfInfo::parse(&body).is_some() {
                            return Ok::<(), io::Error>(());
                        }
                    }
                }
                () = &mut again, if !asked_again => {
                    asked_again = true;
                    ask(stream)?;
                }
            }
        }
    })
    .await;
    match answered {
        Ok(result) => result.map(|()| LoraFirmware::MeshCore),
        Err(_) => Ok(LoraFirmware::Meshtastic),
    }
}

/// Marks the link connected, so frames go out, and queues our own frames
/// that waited for it: messages written before the device was connected
/// would otherwise wait for another link.
fn now_connected(shared: &Weak<Shared>) {
    let Some(node) = shared.upgrade() else {
        return;
    };
    node.lora.connected.store(true, Ordering::Relaxed);
    node.emit(Event::LoraChanged { connected: true });
    let pending = node.engine().pending_own(now(), PENDING_RECEIPTS);
    for frame in pending.unwrap_or_default() {
        if node.lora.send_pending(&frame) {
            node.handed_off(&frame, Via::Lora, 0);
        }
    }
}

/// What the device reported about its radio.
struct Radio {
    modulation: Modulation,
    hop_limit: u32,
}

/// One connection to a Meshtastic device: asks for the configuration, then
/// sends from the queue within the airtime budget and takes in the packets
/// the device hears.
async fn meshtastic_session(shared: &Weak<Shared>, mut stream: Stream) -> io::Result<()> {
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

    now_connected(shared);
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

/// Takes in a packet a Meshtastic device heard, if it carries CarryChat.
fn take(shared: &Weak<Shared>, packet: MeshPacket) {
    let Some(mesh_packet::Payload::Decoded(data)) = packet.payload else {
        return;
    };
    if data.portnum == meshtastic::PRIVATE_APP {
        take_frame(shared, &data.payload);
    }
}

/// Takes in a CarryChat frame the device heard. It is handled like a frame
/// from any link, and passed on to every link but LoRa: the mesh already
/// floods it there.
fn take_frame(shared: &Weak<Shared>, bytes: &[u8]) {
    let Some(shared) = shared.upgrade() else {
        return;
    };
    // A frame that does not parse is skipped, as on every link; file chunks
    // never belong on LoRa.
    let frame = match Frame::from_bytes(bytes) {
        Ok(Frame::Chunk(_)) | Err(_) => return,
        Ok(frame) => frame,
    };
    lock(&shared.lora.seen).carrychat_at = Some(now());
    if let Ok(handled) = shared.handle(frame.clone()) {
        shared.dispatch_from_lora(handled, &frame);
    }
}

/// A MeshCore device, asked one command at a time, with what it pushes
/// meanwhile taken in.
struct Companion {
    shared: Weak<Shared>,
    stream: Stream,
    deframer: meshcore::Deframer,
    /// The device said messages are waiting, or it is time to ask.
    messages_waiting: bool,
}

impl Companion {
    fn send(&self, body: &[u8]) -> io::Result<()> {
        self.stream
            .outgoing
            .send(meshcore::frame(body))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
    }

    /// The next frame from the device, whatever it is.
    async fn next_frame(&mut self) -> io::Result<Vec<u8>> {
        loop {
            if let Some(body) = self.deframer.next_frame() {
                return Ok(body);
            }
            let bytes = self
                .stream
                .incoming
                .recv()
                .await
                .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
            self.deframer.push(&bytes);
        }
    }

    /// The device's next reply of one of the `expected` kinds, or an
    /// error, taking in what it pushes before it. Other replies are late
    /// answers to something asked before, such as the second answer to the
    /// question that found out what the device is, and are skipped.
    async fn reply(&mut self, expected: &[u8]) -> io::Result<Vec<u8>> {
        tokio::time::timeout(REPLY_TIMEOUT, async {
            loop {
                let body = self.next_frame().await?;
                if body[0] >= meshcore::PUSH_FIRST {
                    self.pushed(&body);
                } else if body[0] == meshcore::RESP_ERR || expected.contains(&body[0]) {
                    return Ok(body);
                }
            }
        })
        .await
        .unwrap_or_else(|_| Err(io::Error::other("the device did not answer")))
    }

    async fn ask(&mut self, body: &[u8], expected: &[u8]) -> io::Result<Vec<u8>> {
        self.send(body)?;
        self.reply(expected).await
    }

    /// Takes in what the device pushes: devices heard, and that messages
    /// are waiting.
    fn pushed(&mut self, body: &[u8]) {
        let Some(node) = self.shared.upgrade() else {
            return;
        };
        match body[0] {
            meshcore::PUSH_MSG_WAITING => self.messages_waiting = true,
            meshcore::PUSH_ADVERT if body.len() >= 33 => {
                lock(&node.lora.seen).advert(meshcore::key_number(&body[1..33]), now());
            }
            meshcore::PUSH_NEW_ADVERT => {
                if let Some(contact) = Contact::parse(body) {
                    lock(&node.lora.seen).contact(&contact, now());
                }
            }
            _ => {}
        }
    }

    /// The slot of CarryChat's channel, which is added to the first free
    /// slot if the device does not have it yet. None when every slot is
    /// taken.
    async fn carrychat_channel(&mut self, slots: u8) -> io::Result<Option<u8>> {
        let secret = meshcore::channel_secret();
        let mut free = None;
        for index in 0..slots {
            let reply = self
                .ask(
                    &[meshcore::CMD_GET_CHANNEL, index],
                    &[meshcore::RESP_CHANNEL_INFO],
                )
                .await?;
            let Some(channel) = meshcore::ChannelInfo::parse(&reply) else {
                continue;
            };
            if channel.secret == secret {
                return Ok(Some(index));
            }
            if channel.is_free() && free.is_none() {
                free = Some(index);
            }
        }
        let Some(index) = free else {
            return Ok(None);
        };
        let reply = self
            .ask(
                &meshcore::set_channel(index, meshcore::CHANNEL_NAME, &secret),
                &[meshcore::RESP_OK],
            )
            .await?;
        Ok((reply[0] == meshcore::RESP_OK).then_some(index))
    }

    /// Takes in the devices the device knows.
    async fn contacts(&mut self) -> io::Result<()> {
        let reply = self
            .ask(
                &[meshcore::CMD_GET_CONTACTS],
                &[meshcore::RESP_CONTACTS_START],
            )
            .await?;
        if reply[0] != meshcore::RESP_CONTACTS_START {
            return Ok(());
        }
        loop {
            let reply = self
                .reply(&[meshcore::RESP_CONTACT, meshcore::RESP_END_OF_CONTACTS])
                .await?;
            let Some(contact) = Contact::parse(&reply) else {
                return Ok(());
            };
            if let Some(node) = self.shared.upgrade() {
                lock(&node.lora.seen).contact(&contact, now());
            }
        }
    }

    /// Fetches the messages waiting on the device and takes in CarryChat's
    /// datagrams on `channel`. Other messages are meant for MeshCore's own
    /// apps, and are dropped.
    async fn sync(&mut self, channel: Option<u8>) -> io::Result<()> {
        self.messages_waiting = false;
        loop {
            let reply = self
                .ask(&[meshcore::CMD_SYNC_NEXT_MESSAGE], meshcore::MESSAGES)
                .await?;
            match reply[0] {
                meshcore::RESP_NO_MORE_MESSAGES | meshcore::RESP_ERR => return Ok(()),
                meshcore::RESP_CHANNEL_DATA => {
                    let Some(data) = ChannelData::parse(&reply) else {
                        continue;
                    };
                    if Some(data.channel) == channel && data.data_type == meshcore::DATA_TYPE {
                        take_frame(&self.shared, &data.data);
                    }
                }
                _ => {}
            }
        }
    }
}

/// One connection to a MeshCore device: learns about it, sets its clock,
/// finds or adds CarryChat's channel, then floods frames from the queue on
/// it within the airtime budget and takes in the datagrams heard there.
async fn meshcore_session(shared: &Weak<Shared>, stream: Stream) -> io::Result<()> {
    let mut device = Companion {
        shared: shared.clone(),
        stream,
        deframer: meshcore::Deframer::new(meshcore::FROM_DEVICE),
        messages_waiting: true,
    };
    let info = device
        .ask(
            &[meshcore::CMD_DEVICE_QUERY, meshcore::PROTOCOL_VERSION],
            &[meshcore::RESP_DEVICE_INFO],
        )
        .await?;
    let slots = meshcore::channel_count(&info);
    let me = SelfInfo::parse(
        &device
            .ask(&meshcore::app_start(), &[meshcore::RESP_SELF_INFO])
            .await?,
    )
    .ok_or_else(|| io::Error::other("the device did not say what it is"))?;
    let mut set_time = vec![meshcore::CMD_SET_DEVICE_TIME];
    set_time.extend_from_slice(&now().to_le_bytes());
    device.ask(&set_time, &[meshcore::RESP_OK]).await?;
    let channel = device.carrychat_channel(slots).await?;
    if let Some(node) = shared.upgrade() {
        let mut seen = lock(&node.lora.seen);
        let key = meshcore::key_number(&me.public_key);
        seen.me = Some(key);
        seen.nodes.entry(key).or_default().name = Some(me.name.clone());
        seen.radio = Some(me.radio());
        seen.no_free_channel = channel.is_none();
    }
    device.contacts().await?;
    let modulation = Modulation {
        spread_factor: u32::from(me.spread_factor),
        bandwidth_hz: me.bandwidth_hz,
        coding_rate: u32::from(me.coding_rate.clamp(5, 8) - 4),
    };

    now_connected(shared);
    let Some(wake) = shared.upgrade().map(|node| Arc::clone(&node.lora.wake)) else {
        return Ok(());
    };
    let mut budget = Budget::new();
    let mut sync = tokio::time::interval(SYNC_INTERVAL);
    let mut contacts = tokio::time::interval(CONTACTS_INTERVAL);
    contacts.tick().await;
    loop {
        if device.messages_waiting {
            device.sync(channel).await?;
            continue;
        }
        let Some(node) = shared.upgrade() else {
            return Ok(());
        };
        let airtime = |len: usize| modulation.airtime(len + meshcore::OVERHEAD);
        // Without CarryChat's channel, frames wait.
        let next_len = channel.and(lock(&node.lora.queue).peek_len());
        let wait = next_len.and_then(|len| budget.wait_for(airtime(len)));
        let ready = next_len.is_some() && wait.is_none();
        let packet = if ready {
            lock(&node.lora.queue).next()
        } else {
            None
        };
        drop(node);
        if let (Some(packet), Some(channel)) = (packet, channel) {
            // Queued for a Meshtastic device before this one answered.
            if packet.len() > meshcore::MTU {
                continue;
            }
            let reply = device
                .ask(
                    &meshcore::send_channel_data(channel, &packet),
                    &[meshcore::RESP_OK],
                )
                .await?;
            if reply[..] == [meshcore::RESP_ERR, meshcore::ERR_TABLE_FULL] {
                if let Some(node) = shared.upgrade() {
                    lock(&node.lora.queue).put_back(packet);
                }
                tokio::time::sleep(BUSY_DELAY).await;
            } else {
                budget.spend(airtime(packet.len()));
            }
            continue;
        }
        tokio::select! {
            body = device.next_frame() => {
                let body = body?;
                if body[0] >= meshcore::PUSH_FIRST {
                    device.pushed(&body);
                }
            }
            () = wake.notified() => {}
            () = tokio::time::sleep(wait.unwrap_or_default()), if wait.is_some() => {}
            _ = sync.tick() => device.messages_waiting = true,
            _ = contacts.tick() => device.contacts().await?,
        }
    }
}

/// The bundle or tombstone a frame carries.
fn queued_id(frame: &Frame) -> Option<BundleId> {
    match frame {
        Frame::Bundle(bundle) => Some(bundle.id),
        Frame::Tombstone { tombstone, .. } => Some(tombstone.id),
        Frame::Fragment(_) | Frame::Chunk(_) => None,
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

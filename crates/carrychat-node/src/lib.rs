//! A CarryChat node: identity, contacts, message history and the store-and-forward
//! pool on disk, plus the links that move bundles. Apps (the desktop app, the
//! CLI) talk to a [`Node`] and listen to its [`Event`]s.

#[cfg(feature = "bluetooth")]
mod bluetooth;
pub mod engine;
mod error;
pub mod file;
pub mod internet;
mod journey;
pub mod lan;
pub mod lora;
mod nearby;
pub mod store;
mod sync;

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use carrychat_proto::frame::Frame;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::engine::{Engine, Handled, Outcome};
pub use crate::error::{Error, Result};
pub use crate::internet::{InternetConfig, TorState};
pub use crate::journey::{Journey, Step, Via};
pub use crate::lan::{LanConfig, LanProblem};
pub use crate::lora::{LoraDevice, LoraNode, LoraPort, LoraStatus};
pub use crate::nearby::NearbyPerson;
use crate::store::Store;
pub use crate::sync::Link;

const DATABASE_FILE: &str = "carrychat.db";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub name: String,
    pub fingerprint: String,
    /// The `carrychat:` card text to give to contacts.
    pub card: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactView {
    pub id: i64,
    pub name: String,
    pub fingerprint: String,
    pub last_message: Option<String>,
    pub last_at: Option<u32>,
    /// Wrote first and waits to be accepted or declined.
    pub request: bool,
    /// Messages from them we have not read.
    pub unread: usize,
    /// Our messages to them that have not arrived yet.
    pub waiting: usize,
    /// Whether a ratchet session runs both ways (§5.6): forward secrecy.
    pub secure: bool,
    /// Whether they have our card, so they can join our groups.
    pub has_our_card: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    pub id: i64,
    pub contact_id: i64,
    pub outgoing: bool,
    pub body: String,
    pub created_at: u32,
    /// For outgoing messages: whether the recipient's tombstone came back.
    pub delivered: bool,
    /// For outgoing messages: whether the recipient read it, if they send
    /// read receipts. For incoming ones: whether we read it.
    pub read: bool,
    /// The file it offers, for a photo or voice message.
    pub file: Option<FileView>,
    /// For outgoing messages: the links it has left this device by so far,
    /// in the order first used.
    pub via: Vec<Via>,
}

/// A file offered in a message (§5.9), and how much of it is here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileView {
    pub id: i64,
    pub mime: String,
    pub size: u32,
    pub chunks: u32,
    pub received: u32,
    /// Whether all of it is here, ready to show.
    pub ready: bool,
    /// After this, missing chunks will not come any more.
    pub expires_at: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub id: i64,
    pub name: String,
    /// Whether we are its admin, who alone changes its members.
    pub admin: bool,
    /// Whether we are still in it.
    pub member: bool,
    pub members: Vec<MemberView>,
    pub last_message: Option<String>,
    pub last_at: Option<u32>,
    pub unread: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberView {
    pub name: String,
    pub fingerprint: String,
    pub me: bool,
    pub admin: bool,
    /// Their contact id, if they are one of our contacts.
    pub contact_id: Option<i64>,
    /// Their `carrychat:` card, as the admin passed it on: adding it as a
    /// contact lets us write to them directly.
    pub card: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMessageView {
    pub id: i64,
    pub group_id: i64,
    pub outgoing: bool,
    /// The sender's name, for messages from other members.
    pub sender: Option<String>,
    pub body: String,
    pub created_at: u32,
    /// Whether we read it. Our own messages count as read.
    pub read: bool,
    pub file: Option<FileView>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelView {
    pub id: i64,
    /// The normalized name, without `#`.
    pub name: String,
    pub has_password: bool,
    /// The name we post under.
    pub nick: String,
    pub last_message: Option<String>,
    pub last_at: Option<u32>,
    pub unread: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostView {
    pub id: i64,
    pub channel_id: i64,
    pub outgoing: bool,
    pub nick: String,
    /// A short id of the author's key in this channel, which tells apart
    /// authors with the same nickname.
    pub author: String,
    /// The text, empty for a shared card.
    pub body: String,
    /// The author's contact card, if they shared it.
    pub shared_card: Option<SharedCardView>,
    pub created_at: u32,
    pub read: bool,
    pub file: Option<FileView>,
}

/// An author muted in a channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutedView {
    /// The short id their posts show.
    pub author: String,
    /// The nickname of their latest post we have.
    pub nick: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedCardView {
    pub name: String,
    pub fingerprint: String,
    /// The `carrychat:` card text, to add them as a contact.
    pub card: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Whether contacts learn when we read their messages.
    pub read_receipts: bool,
    /// Whether the internet link goes over Tor. It takes effect when the
    /// link starts.
    pub tor: bool,
    /// Whether the node uses Bluetooth LE. It takes effect when the app starts.
    pub bluetooth: bool,
    /// Whether people nearby who turned it on too can see us, and we them.
    pub nearby: bool,
    /// The Meshtastic device chosen for LoRa: `tcp:host:port`, a serial port
    /// or `bluetooth:address`.
    pub lora_device: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub lan_peers: usize,
    pub lan_port: Option<u16>,
    /// Why the local network link is not working, if it is not.
    pub lan_problem: Option<LanProblem>,
    /// Whether the node is set to use Bluetooth LE.
    pub bluetooth_on: bool,
    /// Nodes connected over Bluetooth LE right now.
    pub bluetooth_peers: usize,
    /// Whether this device's Bluetooth is turned on, once the platform has
    /// said.
    pub bluetooth_radio: Option<bool>,
    /// Nodes connected over Wi-Fi Aware right now, phone to phone.
    pub wifi_aware_peers: usize,
    /// Whether a Meshtastic device is connected for the LoRa link.
    pub lora_connected: bool,
    /// Frames waiting for airtime on LoRa.
    pub lora_waiting: usize,
    /// Internet relays connected right now.
    pub internet_relays: usize,
    /// Whether the internet link was started.
    pub internet_on: bool,
    /// The last reason a relay gave for refusing an event.
    pub internet_refusal: Option<String>,
    /// Tor's state, when the internet link goes over Tor.
    pub internet_tor: Option<TorState>,
    /// Own messages not delivered yet.
    pub outbox: usize,
    /// Bundles carried for other people.
    pub carried: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub received: usize,
    pub delivered: usize,
    pub carried: usize,
    pub skipped: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    #[serde(rename_all = "camelCase")]
    MessageReceived { message: MessageView },
    #[serde(rename_all = "camelCase")]
    MessageDelivered { contact_id: i64, message_id: i64 },
    /// One of our messages left this device by a link it had not used yet.
    #[serde(rename_all = "camelCase")]
    MessageMoved { contact_id: i64, message_id: i64 },
    #[serde(rename_all = "camelCase")]
    PeersChanged {
        lan: usize,
        bluetooth: usize,
        wifi_aware: usize,
    },
    /// Someone nearby became visible, or went away.
    NearbyChanged { people: usize },
    /// The Meshtastic device of the LoRa link connected or went away.
    LoraChanged { connected: bool },
    /// The contact read some of our messages.
    #[serde(rename_all = "camelCase")]
    MessagesRead {
        contact_id: i64,
        message_ids: Vec<i64>,
    },
    /// A contact changed their name, or forward secrecy started with them.
    #[serde(rename_all = "camelCase")]
    ContactChanged { contact: ContactView },
    /// Someone who has our card wrote to us for the first time.
    #[serde(rename_all = "camelCase")]
    ContactRequest { contact: ContactView },
    #[serde(rename_all = "camelCase")]
    GroupMessageReceived { message: GroupMessageView },
    /// We joined or left a group, or its name or members changed.
    #[serde(rename_all = "camelCase")]
    GroupChanged { group: GroupView },
    /// A post in one of our channels.
    #[serde(rename_all = "camelCase")]
    PostReceived { post: PostView },
    /// More of a file arrived, or all of it.
    #[serde(rename_all = "camelCase")]
    FileChanged { file: FileView },
    /// An internet relay connected or disconnected.
    #[serde(rename_all = "camelCase")]
    InternetChanged { relays: usize },
    /// The local network link started or stopped working.
    #[serde(rename_all = "camelCase")]
    LocalNetworkChanged { problem: Option<LanProblem> },
}

#[derive(Clone)]
pub struct Node {
    shared: Arc<Shared>,
}

pub(crate) struct Shared {
    engine: Mutex<Engine>,
    events: broadcast::Sender<Event>,
    pub(crate) lan: lan::Lan,
    pub(crate) peers: sync::Peers,
    pub(crate) lora: lora::Lora,
    pub(crate) internet: internet::Internet,
    internet_on: std::sync::atomic::AtomicBool,
    /// Whether this device's Bluetooth is on: 0 not known yet, 1 off, 2 on.
    bluetooth_radio: std::sync::atomic::AtomicU8,
    pub(crate) nearby: nearby::Nearby,
}

impl Node {
    /// Creates a node with a new identity in `dir`.
    pub fn create(dir: &Path, name: &str) -> Result<Self> {
        if Self::open(dir)?.is_some() {
            return Err(Error::Invalid("a CarryChat identity already exists here"));
        }
        Ok(Self::new(Engine::create(open_store(dir)?, name)?))
    }

    /// Opens the node in `dir`, or returns `None` if there is none yet.
    pub fn open(dir: &Path) -> Result<Option<Self>> {
        if !dir.join(DATABASE_FILE).exists() {
            return Ok(None);
        }
        Ok(Engine::load(open_store(dir)?)?.map(Self::new))
    }

    fn new(engine: Engine) -> Self {
        let visible = engine.settings().is_ok_and(|settings| settings.nearby);
        Self {
            shared: Arc::new(Shared {
                engine: Mutex::new(engine),
                events: broadcast::channel(256).0,
                lan: lan::Lan::new(),
                peers: sync::Peers::new(),
                lora: lora::Lora::new(),
                internet: internet::Internet::new(),
                internet_on: std::sync::atomic::AtomicBool::new(false),
                bluetooth_radio: std::sync::atomic::AtomicU8::new(0),
                nearby: nearby::Nearby::new(visible),
            }),
        }
    }

    pub fn profile(&self) -> Profile {
        self.shared.engine().profile()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.shared.events.subscribe()
    }

    pub fn add_contact(&self, card_text: &str) -> Result<ContactView> {
        let (contact, opened) = self
            .shared
            .engine()
            .atomically(|engine| engine.add_contact(card_text, now()))?;
        for handled in opened {
            self.shared.dispatch(handled, None);
        }
        self.shared.internet.renew_subscription();
        Ok(contact)
    }

    /// Marks a contact's messages read and, unless read receipts are off,
    /// tells them.
    pub fn mark_read(&self, contact_id: i64) -> Result<()> {
        let receipts = self
            .shared
            .engine()
            .atomically(|engine| engine.mark_read(contact_id, now()))?;
        self.shared.send_own(&receipts);
        Ok(())
    }

    /// Changes our name and sends the new card to our contacts.
    pub fn rename(&self, name: &str) -> Result<Profile> {
        let (profile, updates) = self
            .shared
            .engine()
            .atomically(|engine| engine.rename(name, now()))?;
        self.shared.send_own(&updates);
        Ok(profile)
    }

    pub fn settings(&self) -> Result<Settings> {
        self.shared.engine().settings()
    }

    pub fn set_read_receipts(&self, on: bool) -> Result<()> {
        self.shared.engine().set_read_receipts(on)
    }

    /// Becomes visible to people nearby who are visible too, or stops. It
    /// applies at once and is kept for the next start.
    pub fn set_nearby(&self, on: bool) -> Result<()> {
        self.shared.engine().set_nearby(on)?;
        self.shared.set_nearby_visible(on);
        Ok(())
    }

    /// The people nearby who are visible, while this node is.
    pub fn nearby(&self) -> Vec<NearbyPerson> {
        self.shared.nearby_people()
    }

    /// Chooses whether the internet link goes over Tor from its next start.
    pub fn set_tor(&self, on: bool) -> Result<()> {
        self.shared.engine().set_tor(on)
    }

    /// Chooses whether the node uses Bluetooth LE from its next start.
    pub fn set_bluetooth(&self, on: bool) -> Result<()> {
        self.shared.engine().set_bluetooth(on)
    }

    /// Accepts a request: its sender becomes a contact.
    pub fn accept_contact(&self, contact_id: i64) -> Result<ContactView> {
        let contact = self
            .shared
            .engine()
            .atomically(|engine| engine.accept_contact(contact_id))?;
        self.shared.internet.renew_subscription();
        Ok(contact)
    }

    /// Declines a request and ignores its sender from now on.
    pub fn decline_contact(&self, contact_id: i64) -> Result<()> {
        self.shared
            .engine()
            .atomically(|engine| engine.decline_contact(contact_id))?;
        self.shared.internet.renew_subscription();
        Ok(())
    }

    pub fn contacts(&self) -> Result<Vec<ContactView>> {
        self.shared.engine().contacts()
    }

    pub fn messages(&self, contact_id: i64) -> Result<Vec<MessageView>> {
        self.shared.engine().messages(contact_id)
    }

    /// Where one of our messages went, from writing to reading.
    pub fn journey(&self, message_id: i64) -> Result<Journey> {
        self.shared.engine().journey(message_id)
    }

    /// Sends a text message over every link that is up, and keeps it until
    /// it is delivered or expires.
    pub fn send_text(&self, contact_id: i64, text: &str) -> Result<MessageView> {
        let (message, frame, tag) = {
            let mut engine = self.shared.engine();
            let (message, frame) =
                engine.atomically(|engine| engine.send_text(contact_id, text, now()))?;
            let tag = match &frame {
                Frame::Bundle(bundle) => engine.publish_tag(bundle)?,
                _ => unreachable!("sending makes a bundle"),
            };
            (message, frame, tag)
        };
        self.shared.send_own_frames(&[(frame, tag)]);
        self.shared.internet.renew_subscription();
        Ok(message)
    }

    pub fn groups(&self) -> Result<Vec<GroupView>> {
        self.shared.engine().groups()
    }

    pub fn group_messages(&self, group_id: i64) -> Result<Vec<GroupMessageView>> {
        self.shared.engine().group_messages(group_id)
    }

    /// Creates a group with us as its admin and the given contacts in it.
    pub fn create_group(&self, name: &str, contact_ids: &[i64]) -> Result<GroupView> {
        let group = self
            .shared
            .engine()
            .atomically(|engine| engine.create_group(name, contact_ids, now()))?;
        self.shared.flush_own();
        Ok(group)
    }

    pub fn add_group_member(&self, group_id: i64, contact_id: i64) -> Result<GroupView> {
        let group = self
            .shared
            .engine()
            .atomically(|engine| engine.add_group_member(group_id, contact_id, now()))?;
        self.shared.flush_own();
        Ok(group)
    }

    /// Removes a member, named by fingerprint.
    pub fn remove_group_member(&self, group_id: i64, fingerprint: &str) -> Result<GroupView> {
        let group = self
            .shared
            .engine()
            .atomically(|engine| engine.remove_group_member(group_id, fingerprint, now()))?;
        self.shared.flush_own();
        Ok(group)
    }

    pub fn leave_group(&self, group_id: i64) -> Result<GroupView> {
        let group = self
            .shared
            .engine()
            .atomically(|engine| engine.leave_group(group_id, now()))?;
        self.shared.flush_own();
        Ok(group)
    }

    /// Sends a text message to a group: one bundle that every member opens.
    pub fn send_group_text(&self, group_id: i64, text: &str) -> Result<GroupMessageView> {
        let (message, frame) = self
            .shared
            .engine()
            .atomically(|engine| engine.send_group_text(group_id, text, now()))?;
        self.shared.send_own_post(&frame);
        Ok(message)
    }

    pub fn mark_group_read(&self, group_id: i64) -> Result<()> {
        self.shared.engine().mark_group_read(group_id, now())
    }

    pub fn channels(&self) -> Result<Vec<ChannelView>> {
        self.shared.engine().channels()
    }

    /// Joins a public channel, posting under `nick`.
    pub fn join_channel(&self, name: &str, password: &str, nick: &str) -> Result<ChannelView> {
        let (channel, opened) = self
            .shared
            .engine()
            .atomically(|engine| engine.join_channel(name, password, nick, now()))?;
        for handled in opened {
            self.shared.dispatch(handled, None);
        }
        self.shared.internet.renew_subscription();
        Ok(channel)
    }

    pub fn leave_channel(&self, channel_id: i64) -> Result<()> {
        self.shared
            .engine()
            .atomically(|engine| engine.leave_channel(channel_id))?;
        self.shared.internet.renew_subscription();
        Ok(())
    }

    pub fn set_channel_nick(&self, channel_id: i64, nick: &str) -> Result<ChannelView> {
        self.shared.engine().set_channel_nick(channel_id, nick)
    }

    pub fn posts(&self, channel_id: i64) -> Result<Vec<PostView>> {
        self.shared.engine().posts(channel_id)
    }

    /// Posts text to a channel.
    pub fn send_post(&self, channel_id: i64, text: &str) -> Result<PostView> {
        let (post, frame) = self
            .shared
            .engine()
            .atomically(|engine| engine.send_post(channel_id, text, now()))?;
        self.shared.send_own_post(&frame);
        Ok(post)
    }

    /// Posts our contact card to a channel, so readers can add us.
    pub fn share_card(&self, channel_id: i64) -> Result<PostView> {
        let (post, frame) = self
            .shared
            .engine()
            .atomically(|engine| engine.share_card(channel_id, now()))?;
        self.shared.send_own_post(&frame);
        Ok(post)
    }

    /// Mutes an author, by the short id their posts show.
    pub fn mute_author(&self, channel_id: i64, author: &str) -> Result<()> {
        self.shared.engine().mute_author(channel_id, author)
    }

    /// Shows a muted author's posts again. Posts dropped while they were
    /// muted are asked for again from the internet relays.
    pub fn unmute_author(&self, channel_id: i64, author: &str) -> Result<()> {
        self.shared.engine().unmute_author(channel_id, author)?;
        self.shared.internet.renew_subscription();
        Ok(())
    }

    pub fn muted_authors(&self, channel_id: i64) -> Result<Vec<MutedView>> {
        self.shared.engine().muted_authors(channel_id)
    }

    pub fn mark_channel_read(&self, channel_id: i64) -> Result<()> {
        self.shared.engine().mark_channel_read(channel_id, now())
    }

    /// Sends a file to a contact: a photo or a voice message.
    pub fn send_file(&self, contact_id: i64, mime: &str, data: &[u8]) -> Result<MessageView> {
        let (message, frames) = self
            .shared
            .engine()
            .atomically(|engine| engine.send_file(contact_id, mime, data, now()))?;
        self.shared.send_own_frames(&frames);
        Ok(message)
    }

    pub fn send_group_file(
        &self,
        group_id: i64,
        mime: &str,
        data: &[u8],
    ) -> Result<GroupMessageView> {
        let (message, frames) = self
            .shared
            .engine()
            .atomically(|engine| engine.send_group_file(group_id, mime, data, now()))?;
        self.shared.send_own_frames(&frames);
        Ok(message)
    }

    pub fn send_channel_file(&self, channel_id: i64, mime: &str, data: &[u8]) -> Result<PostView> {
        let (post, frames) = self
            .shared
            .engine()
            .atomically(|engine| engine.send_channel_file(channel_id, mime, data, now()))?;
        self.shared.send_own_frames(&frames);
        Ok(post)
    }

    /// A file's media type and bytes, once all of it is here.
    pub fn file_data(&self, file_id: i64) -> Result<Option<(String, Vec<u8>)>> {
        self.shared.engine().file_data(file_id)
    }

    pub fn status(&self) -> Result<Status> {
        let (outbox, carried) = self.shared.engine().counts()?;
        Ok(Status {
            lan_peers: self.shared.peers.count(Link::Lan),
            lan_port: self.shared.lan.port(),
            lan_problem: self.shared.lan.problem(),
            bluetooth_on: self.shared.engine().settings()?.bluetooth,
            bluetooth_peers: self.shared.peers.count(Link::Bluetooth),
            bluetooth_radio: match self
                .shared
                .bluetooth_radio
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                1 => Some(false),
                2 => Some(true),
                _ => None,
            },
            wifi_aware_peers: self.shared.peers.count(Link::WifiAware),
            lora_connected: self.shared.lora.connected(),
            lora_waiting: self.shared.lora.waiting(),
            internet_relays: self.shared.internet.connected(),
            internet_on: self
                .shared
                .internet_on
                .load(std::sync::atomic::Ordering::Relaxed),
            internet_refusal: self.shared.internet.refusal(),
            internet_tor: self.shared.internet.tor_state(),
            outbox,
            carried,
        })
    }

    /// Writes every bundle and tombstone this node holds to a file, for
    /// someone to carry. Returns the number of frames written.
    pub fn export_file(&self, path: &Path) -> Result<usize> {
        let (data, count) = self.export_bytes()?;
        std::fs::write(path, data)?;
        Ok(count)
    }

    /// What `export_file` writes, and how many frames it holds, for a
    /// caller that writes it somewhere other than a path.
    pub fn export_bytes(&self) -> Result<(Vec<u8>, usize)> {
        let frames = self.shared.engine().export_frames(now())?;
        for frame in &frames {
            self.shared.handed_off(frame, Via::File, 0);
        }
        Ok((file::encode(&frames), frames.len()))
    }

    pub fn import_file(&self, path: &Path) -> Result<ImportReport> {
        if std::fs::metadata(path)?.len() > file::MAX_FILE_LEN {
            return Err(Error::Invalid("file is too large"));
        }
        self.import_bytes(&std::fs::read(path)?)
    }

    /// Takes in a file that `export_bytes` wrote.
    pub fn import_bytes(&self, data: &[u8]) -> Result<ImportReport> {
        if data.len() as u64 > file::MAX_FILE_LEN {
            return Err(Error::Invalid("file is too large"));
        }
        let (frames, skipped) = file::decode(data)?;
        let mut report = ImportReport {
            skipped,
            ..ImportReport::default()
        };
        for frame in frames {
            let handled = self.shared.handle(frame)?;
            match handled.outcome {
                Outcome::Received => report.received += 1,
                Outcome::Delivered => report.delivered += 1,
                Outcome::Carried => report.carried += 1,
                _ => {}
            }
            self.shared.dispatch(handled, None);
        }
        Ok(report)
    }

    /// Starts the local network link and housekeeping. Returns the TCP port
    /// other nodes can connect to.
    pub async fn start(&self, config: LanConfig) -> Result<u16> {
        let port = lan::start(&self.shared, config).await?;
        let shared = Arc::downgrade(&self.shared);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tick.tick().await;
                let Some(shared) = shared.upgrade() else {
                    return;
                };
                let _ = shared.engine().prune(now());
            }
        });
        Ok(port)
    }

    /// Starts the internet link: connects to the relays and keeps doing so.
    pub fn start_internet(&self, config: InternetConfig) -> Result<()> {
        internet::start(&self.shared, config)?;
        self.shared
            .internet_on
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Connects to a node at a known address, for networks where discovery
    /// does not work.
    pub async fn connect(&self, addr: std::net::SocketAddr) -> Result<()> {
        lan::dial(&self.shared, addr, None).await
    }

    /// Starts the LoRa link (§9.2) through a Meshtastic device, in place of
    /// any other, connecting again whenever the connection ends.
    pub fn start_lora(&self, device: LoraDevice) -> Result<()> {
        lora::start(&self.shared, device)
    }

    /// Uses `device` for LoRa from now on, or none, and keeps the choice
    /// for the next start, which reads it from the settings.
    pub fn set_lora(&self, device: Option<LoraDevice>) -> Result<()> {
        let text = device.as_ref().map(ToString::to_string);
        self.shared.engine().set_lora_device(text.as_deref())?;
        match device {
            Some(device) => lora::start(&self.shared, device),
            None => {
                lora::stop(&self.shared);
                Ok(())
            }
        }
    }

    /// The device in use for LoRa, if any.
    pub fn lora_device(&self) -> Option<LoraDevice> {
        self.shared.lora.device()
    }

    /// How the LoRa link stands: the device, its region and what it hears.
    pub fn lora(&self) -> LoraStatus {
        self.shared.lora.status(now())
    }

    /// Runs the LoRa link over a connection platform code made to the
    /// chosen device, such as Bluetooth LE on Android, carrying the stream
    /// framing a serial port does. Returns when the connection ends.
    pub async fn lora_over<R, W>(&self, reader: R, writer: W) -> Result<()>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        lora::over(&self.shared, reader, writer).await
    }

    /// Starts looking for nearby nodes over Bluetooth LE and connecting to
    /// them (§9.5), where btleplug supports it.
    #[cfg(feature = "bluetooth")]
    pub async fn start_bluetooth(&self) -> Result<()> {
        bluetooth::start(&self.shared).await
    }

    /// Notes whether this device's Bluetooth is on, for platform code that
    /// runs the Bluetooth LE link itself (§9.5).
    pub fn set_bluetooth_radio(&self, on: bool) {
        self.shared.set_bluetooth_radio(on);
    }

    /// The id this run of the node goes by on nearby links, for platform
    /// code that advertises the node over Bluetooth LE (§9.5).
    pub fn session(&self) -> u64 {
        self.shared.peers.session()
    }

    /// Whether to connect to a node seen nearby with this session id. A yes
    /// counts as a connection in progress, which `sync_over` ends, or
    /// `dial_ended` if the connection could not be made.
    pub fn should_connect(&self, peer: u64) -> bool {
        self.shared.peers.should_dial(peer, false)
    }

    pub fn dial_ended(&self, peer: u64) {
        self.shared.peers.dial_ended(peer);
    }

    /// Syncs over a connection that platform code opened, such as a
    /// Bluetooth LE connection on Android, until it ends. `dialed` is the
    /// session id of the node this side connected to, if it connected.
    pub async fn sync_over<R, W>(&self, link: Link, reader: R, writer: W, dialed: Option<u64>)
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        sync::run(self.shared.clone(), link, reader, writer, dialed).await;
    }
}

impl Shared {
    pub(crate) fn engine(&self) -> MutexGuard<'_, Engine> {
        self.engine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Handles a frame atomically: if any part fails, nothing of it is kept.
    pub(crate) fn handle(&self, frame: Frame) -> Result<Handled> {
        self.engine()
            .atomically(|engine| engine.handle(frame, now()))
    }

    /// Passes relayed frames to LAN peers, and events to apps. A bundle is not
    /// sent back to the peer it came from, but a tombstone is: that peer may be
    /// the sender waiting for it.
    pub(crate) fn dispatch(&self, handled: Handled, from: Option<u64>) {
        self.relay(handled, from, None);
    }

    /// Like `dispatch`, for a frame heard over LoRa: it goes to every other
    /// link, but not back on LoRa, where the mesh already floods it (§8).
    /// What it makes the node send, such as its receipt, does go on LoRa.
    pub(crate) fn dispatch_from_lora(&self, handled: Handled, heard: &Frame) {
        self.relay(handled, None, Some(heard));
    }

    fn relay(&self, handled: Handled, from: Option<u64>, heard_on_lora: Option<&Frame>) {
        for frame in &handled.relay {
            let except = match frame {
                Frame::Bundle(_) | Frame::Chunk(_) => from,
                _ => None,
            };
            self.peers.broadcast(&frame.to_bytes(), except);
            if !heard_on_lora.is_some_and(|heard| lora::echoes(frame, heard)) {
                self.lora.send(frame, false);
            }
        }
        self.internet.post_handled(&handled);
        // A bundle posted as it arrives need not go up again on reconnecting.
        if handled.outcome == Outcome::Carried && self.internet.connected() > 0 {
            for frame in &handled.relay {
                if let Frame::Bundle(bundle) | Frame::Chunk(bundle) = frame {
                    let _ = self.engine().mark_uploaded(bundle.id);
                }
            }
        }
        self.flush_own();
        if handled
            .events
            .iter()
            .any(|event| matches!(event, Event::ContactRequest { .. }))
        {
            self.internet.renew_subscription();
        }
        for event in handled.events {
            self.emit(event);
        }
    }

    /// Sends the own bundles the engine made on its own, such as session
    /// starts and group keys.
    fn flush_own(&self) {
        let own = self.engine().take_own();
        self.send_own(&own);
        if self.engine().take_tags_changed() {
            self.internet.renew_subscription();
        }
    }

    /// Sends our new frames, a file's offer and chunks, on every link.
    fn send_own_frames(&self, frames: &[(Frame, carrychat_proto::pair::RecipientTag)]) {
        for (frame, tag) in frames {
            for (peer, link) in self.peers.broadcast(&frame.to_bytes(), None) {
                self.handed_off(frame, link.into(), peer);
            }
            self.internet.post_bundle(frame, tag);
            if self.lora.send(frame, true) {
                self.handed_off(frame, Via::Lora, 0);
            }
        }
        self.internet.probe();
    }

    /// Sends a new post of ours, or a group message, on every link.
    fn send_own_post(&self, frame: &Frame) {
        let Frame::Bundle(bundle) = frame else {
            unreachable!("sending makes a bundle");
        };
        self.peers.broadcast(&frame.to_bytes(), None);
        self.internet.post_bundle(frame, &bundle.tag);
        self.lora.send(frame, true);
        self.internet.probe();
    }

    /// Sends our own new bundles on every link that is up.
    fn send_own(&self, frames: &[(Frame, carrychat_proto::pair::RecipientTag)]) {
        for (frame, tag) in frames {
            self.peers.broadcast(&frame.to_bytes(), None);
            self.internet.post_bundle(frame, tag);
            self.lora.send(frame, true);
        }
        if !frames.is_empty() {
            self.internet.renew_subscription();
        }
    }

    pub(crate) fn set_bluetooth_radio(&self, on: bool) {
        self.bluetooth_radio
            .store(if on { 2 } else { 1 }, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }
}

/// Deletes everything a node keeps in `dir`: identity, contacts, messages
/// and carried bundles. Stop using every [`Node`] opened from `dir` first; a
/// running one keeps what it has in memory until it ends.
pub fn erase(dir: &Path) -> Result<()> {
    for name in [DATABASE_FILE, "carrychat.db-wal", "carrychat.db-shm"] {
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn open_store(dir: &Path) -> Result<Store> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Store::open(&dir.join(DATABASE_FILE))
}

pub(crate) fn now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as u32)
}

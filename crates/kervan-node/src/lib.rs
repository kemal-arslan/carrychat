//! A Kervan node: identity, contacts, message history and the store-and-forward
//! pool on disk, plus the links that move bundles. Apps (the desktop app, the
//! CLI) talk to a [`Node`] and listen to its [`Event`]s.

pub mod engine;
mod error;
pub mod file;
pub mod internet;
pub mod lan;
pub mod store;

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use kervan_proto::frame::Frame;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::engine::{Engine, Handled, Outcome};
pub use crate::error::{Error, Result};
pub use crate::internet::InternetConfig;
pub use crate::lan::{LanConfig, LanProblem};
use crate::store::Store;

const DATABASE_FILE: &str = "kervan.db";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub name: String,
    pub fingerprint: String,
    /// The `kervan:` card text to give to contacts.
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
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub lan_peers: usize,
    pub lan_port: Option<u16>,
    /// Why the local network link is not working, if it is not.
    pub lan_problem: Option<LanProblem>,
    /// Internet relays connected right now.
    pub internet_relays: usize,
    /// Whether the internet link was started.
    pub internet_on: bool,
    /// The last reason a relay gave for refusing an event.
    pub internet_refusal: Option<String>,
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
    #[serde(rename_all = "camelCase")]
    PeersChanged { count: usize },
    /// Someone who has our card wrote to us for the first time.
    #[serde(rename_all = "camelCase")]
    ContactRequest { contact: ContactView },
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
    pub(crate) internet: internet::Internet,
    internet_on: std::sync::atomic::AtomicBool,
}

impl Node {
    /// Creates a node with a new identity in `dir`.
    pub fn create(dir: &Path, name: &str) -> Result<Self> {
        if Self::open(dir)?.is_some() {
            return Err(Error::Invalid("a Kervan identity already exists here"));
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
        Self {
            shared: Arc::new(Shared {
                engine: Mutex::new(engine),
                events: broadcast::channel(256).0,
                lan: lan::Lan::new(),
                internet: internet::Internet::new(),
                internet_on: std::sync::atomic::AtomicBool::new(false),
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
        let (contact, opened) = self.shared.engine().add_contact(card_text, now())?;
        for handled in opened {
            self.shared.dispatch(handled, None);
        }
        self.shared.internet.renew_subscription();
        Ok(contact)
    }

    /// Accepts a request: its sender becomes a contact.
    pub fn accept_contact(&self, contact_id: i64) -> Result<ContactView> {
        let contact = self.shared.engine().accept_contact(contact_id)?;
        self.shared.internet.renew_subscription();
        Ok(contact)
    }

    /// Declines a request and ignores its sender from now on.
    pub fn decline_contact(&self, contact_id: i64) -> Result<()> {
        self.shared.engine().decline_contact(contact_id)?;
        self.shared.internet.renew_subscription();
        Ok(())
    }

    pub fn contacts(&self) -> Result<Vec<ContactView>> {
        self.shared.engine().contacts()
    }

    pub fn messages(&self, contact_id: i64) -> Result<Vec<MessageView>> {
        self.shared.engine().messages(contact_id)
    }

    /// Sends a text message over every link that is up, and keeps it until
    /// it is delivered or expires.
    pub fn send_text(&self, contact_id: i64, text: &str) -> Result<MessageView> {
        let (message, frame, tag) = {
            let mut engine = self.shared.engine();
            let (message, frame) = engine.send_text(contact_id, text, now())?;
            let tag = match &frame {
                Frame::Bundle(bundle) => engine.publish_tag(bundle)?,
                _ => unreachable!("sending makes a bundle"),
            };
            (message, frame, tag)
        };
        self.shared.lan.broadcast(&frame.to_bytes(), None);
        self.shared.internet.post_bundle(&frame, &tag);
        self.shared.internet.renew_subscription();
        Ok(message)
    }

    pub fn status(&self) -> Result<Status> {
        let (outbox, carried) = self.shared.engine().counts()?;
        Ok(Status {
            lan_peers: self.shared.lan.peer_count(),
            lan_port: self.shared.lan.port(),
            lan_problem: self.shared.lan.problem(),
            internet_relays: self.shared.internet.connected(),
            internet_on: self
                .shared
                .internet_on
                .load(std::sync::atomic::Ordering::Relaxed),
            internet_refusal: self.shared.internet.refusal(),
            outbox,
            carried,
        })
    }

    /// Writes every bundle and tombstone this node holds to a file, for
    /// someone to carry. Returns the number of frames written.
    pub fn export_file(&self, path: &Path) -> Result<usize> {
        let frames = self.shared.engine().export_frames(now())?;
        std::fs::write(path, file::encode(&frames))?;
        Ok(frames.len())
    }

    pub fn import_file(&self, path: &Path) -> Result<ImportReport> {
        if std::fs::metadata(path)?.len() > file::MAX_FILE_LEN {
            return Err(Error::Invalid("file is too large"));
        }
        let (frames, skipped) = file::decode(&std::fs::read(path)?)?;
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
}

impl Shared {
    pub(crate) fn engine(&self) -> MutexGuard<'_, Engine> {
        self.engine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn handle(&self, frame: Frame) -> Result<Handled> {
        self.engine().handle(frame, now())
    }

    /// Passes relayed frames to LAN peers, and events to apps. A bundle is not
    /// sent back to the peer it came from, but a tombstone is: that peer may be
    /// the sender waiting for it.
    pub(crate) fn dispatch(&self, handled: Handled, from: Option<u64>) {
        for frame in &handled.relay {
            let except = match frame {
                Frame::Bundle(_) => from,
                _ => None,
            };
            self.lan.broadcast(&frame.to_bytes(), except);
        }
        self.internet.post_handled(&handled);
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

    pub(crate) fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }
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

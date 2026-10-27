//! Where our own messages went: each link a message left this device by,
//! how many devices or relays took it there, and when. Nothing about the
//! way beyond them is known; carriers do not report back.

use carrychat_proto::bundle::BundleId;
use carrychat_proto::frame::Frame;
use serde::Serialize;

use crate::sync::Link;
use crate::{now, Event, Shared};

/// A way a message left this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Via {
    Lan,
    Bluetooth,
    WifiAware,
    WifiDirect,
    Internet,
    Lora,
    File,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Lan => "lan",
            Via::Bluetooth => "bluetooth",
            Via::WifiAware => "wifiAware",
            Via::WifiDirect => "wifiDirect",
            Via::Internet => "internet",
            Via::Lora => "lora",
            Via::File => "file",
        }
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "lan" => Via::Lan,
            "bluetooth" => Via::Bluetooth,
            "wifiAware" => Via::WifiAware,
            "wifiDirect" => Via::WifiDirect,
            "internet" => Via::Internet,
            "lora" => Via::Lora,
            "file" => Via::File,
            _ => return None,
        })
    }
}

impl From<Link> for Via {
    fn from(link: Link) -> Self {
        match link {
            Link::Lan => Via::Lan,
            Link::Bluetooth => Via::Bluetooth,
            Link::WifiAware => Via::WifiAware,
            Link::WifiDirect => Via::WifiDirect,
        }
    }
}

/// One of our messages from writing to reading.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Journey {
    pub created_at: u32,
    pub delivered_at: Option<u32>,
    pub read_at: Option<u32>,
    /// Each link it left by, in the order first used.
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub via: Via,
    /// Devices nearby, or relays on the internet, that took it.
    pub count: usize,
    /// When it first left this way.
    pub at: u32,
}

impl Shared {
    /// Notes that a frame went to `peer` over `via`, if it is one of our
    /// messages still on its way, and tells the app when that is a new way.
    pub(crate) fn handed_off(&self, frame: &Frame, via: Via, peer: u64) {
        let Frame::Bundle(bundle) = frame else {
            return;
        };
        self.handed_off_id(bundle.id, via, peer);
    }

    pub(crate) fn handed_off_id(&self, id: BundleId, via: Via, peer: u64) {
        let moved = self.engine().record_handoff(id, via, peer, now());
        if let Ok(Some((contact_id, message_id))) = moved {
            self.emit(Event::MessageMoved {
                contact_id,
                message_id,
            });
        }
    }
}

//! Light (§9.11): one of this node's messages blinked by a flashlight, and
//! a watcher that reads what a camera sees of another device's flashlight
//! and takes in the frames like those of any link.

use carrychat_proto::frame::Frame;
use carrychat_proto::identity::ContactCard;
use carrychat_proto::light;
use carrychat_proto::sound::Kind;
use serde::Serialize;

use crate::sound::{take_frame, HeardCard};
use crate::Node;

/// The longest frame one round of light carries.
pub(crate) const MTU: usize = light::MAX_FRAME_LEN;

/// What a watch has read so far, and how it goes.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LightWatch {
    /// A card read whole, its signature verified.
    pub card: Option<HeardCard>,
    /// Frames read whole and taken in since the watch began.
    pub frames: u32,
    /// How far apart the light's lit and dark look, 0 to 255; a light
    /// blinking shows above `light::MIN_CONTRAST`.
    pub contrast: f32,
    /// Blocks read of the round being read, and how many it has.
    pub blocks: usize,
    pub of: usize,
}

/// Watches what a camera sees of a flashlight nearby.
#[derive(Default)]
pub struct LightListener {
    listener: light::Listener,
    frames: u32,
}

impl LightListener {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes in pictures: when each was taken, in milliseconds, in order,
    /// and how bright the light looks in it, 0 to 255. Frames read whole go
    /// to `node` as from any link; a card read whole is returned once its
    /// signature verifies.
    pub fn push(&mut self, node: &Node, pictures: &[(f64, f32)]) -> LightWatch {
        let mut card = None;
        for (kind, bytes) in self.listener.push(pictures) {
            match kind {
                Kind::Card => {
                    if let Some(heard) = ContactCard::from_bytes(&bytes)
                        .ok()
                        .and_then(HeardCard::verified)
                    {
                        card = Some(heard);
                    }
                }
                Kind::Frame => {
                    take_frame(&node.shared, &bytes);
                    self.frames += 1;
                }
            }
        }
        let progress = self.listener.progress();
        LightWatch {
            card,
            frames: self.frames,
            contrast: progress.contrast,
            blocks: progress.blocks,
            of: progress.of,
        }
    }
}

/// Frames as rounds of light, one after another: symbols, true for lit,
/// each `light::SYMBOL_MS` long, to blink over and over.
pub(crate) fn frames_pattern(frames: &[Frame]) -> Vec<bool> {
    let bytes: Vec<Vec<u8>> = frames.iter().map(Frame::to_bytes).collect();
    let rounds: Vec<&[u8]> = bytes.iter().map(Vec::as_slice).collect();
    light::patterns(Kind::Frame, &rounds)
}

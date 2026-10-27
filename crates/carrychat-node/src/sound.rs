//! Sound (§3.2, §9.10): this node's card, or one of its messages, played as
//! sound, and a listener that hears another's card, and takes in the
//! frames it hears like those of any link.

use carrychat_proto::frame::Frame;
use carrychat_proto::identity::ContactCard;
use carrychat_proto::sound::{self, Band, Kind};
use serde::Serialize;

use crate::{Node, Result, Shared};

/// The longest frame one round of sound carries.
pub(crate) const MTU: usize = sound::MAX_FRAME_LEN;

/// A card heard over sound or read over NFC, its signature verified.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HeardCard {
    /// The `carrychat:` card text, as it would be pasted.
    pub card: String,
    pub name: String,
}

impl HeardCard {
    /// A card in its `carrychat:` text form, if its signature verifies.
    pub fn from_text(text: &str) -> Option<Self> {
        Self::verified(ContactCard::from_text(text).ok()?)
    }

    fn verified(card: ContactCard) -> Option<Self> {
        Some(Self {
            card: card.to_text(),
            name: card.name().to_owned(),
        })
    }
}

/// Listens to a microphone for what a device nearby plays: a card, or the
/// frames of a message.
pub struct SoundListener(sound::Listener);

impl SoundListener {
    /// A listener for samples at `rate` per second.
    pub fn new(rate: u32) -> Self {
        Self(sound::Listener::new(rate))
    }

    /// Takes in samples, between -1 and 1. A frame heard whole, its check
    /// matching, goes to `node` as from any link. A card heard whole is
    /// returned once its signature verifies; what does not verify was heard
    /// wrong, and listening goes on.
    pub fn push(&mut self, node: &Node, samples: &[f32]) -> Option<HeardCard> {
        let (kind, bytes) = self.0.push(samples)?;
        match kind {
            Kind::Card => HeardCard::verified(ContactCard::from_bytes(&bytes).ok()?),
            Kind::Frame => {
                take_frame(&node.shared, &bytes);
                None
            }
        }
    }
}

/// Takes in a frame heard over sound, handled like a frame from any link
/// and passed on to the links. File chunks never come by sound.
fn take_frame(shared: &Shared, bytes: &[u8]) {
    let frame = match Frame::from_bytes(bytes) {
        Ok(Frame::Chunk(_)) | Err(_) => return,
        Ok(frame) => frame,
    };
    if let Ok(handled) = shared.handle(frame) {
        shared.dispatch(handled, None);
    }
}

/// `card` as one round of sound in `band`, a WAV file to play over and
/// over.
pub(crate) fn card_wav(card: &str, band: Band) -> Result<Vec<u8>> {
    Ok(sound::wav(
        band,
        Kind::Card,
        &[&ContactCard::from_text(card)?.to_bytes()],
    ))
}

/// Frames as rounds of sound in `band`, one after another, in a WAV file.
pub(crate) fn frames_wav(frames: &[Frame], band: Band) -> Vec<u8> {
    let bytes: Vec<Vec<u8>> = frames.iter().map(Frame::to_bytes).collect();
    let rounds: Vec<&[u8]> = bytes.iter().map(Vec::as_slice).collect();
    sound::wav(band, Kind::Frame, &rounds)
}

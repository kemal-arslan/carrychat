//! Cards over sound (§3.2): this node's card played as sound, and a
//! listener that hears another's.

use carrychat_proto::identity::ContactCard;
use carrychat_proto::sound;
use serde::Serialize;

/// A card heard over sound, its signature verified.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HeardCard {
    /// The `carrychat:` card text, as it would be pasted.
    pub card: String,
    pub name: String,
}

/// Listens to a microphone for a card played as sound.
pub struct CardListener(sound::Listener);

impl CardListener {
    /// A listener for samples at `rate` per second.
    pub fn new(rate: u32) -> Self {
        Self(sound::Listener::new(rate))
    }

    /// Takes in samples, between -1 and 1. Returns the card once one has
    /// been heard whole and its signature verifies; what does not verify
    /// was heard wrong, and listening goes on.
    pub fn push(&mut self, samples: &[f32]) -> Option<HeardCard> {
        let bytes = self.0.push(samples)?;
        let card = ContactCard::from_bytes(&bytes).ok()?;
        Some(HeardCard {
            card: card.to_text(),
            name: card.name().to_owned(),
        })
    }
}

/// `card` as one round of sound, a WAV file to play over and over.
pub(crate) fn wav(card: &str) -> crate::Result<Vec<u8>> {
    Ok(sound::wav(&ContactCard::from_text(card)?.to_bytes()))
}

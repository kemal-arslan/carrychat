//! Cards passed between devices side by side: this node's card played as
//! sound and a listener that hears another's (§3.2), and cards read over
//! NFC (§3.3).

use carrychat_proto::identity::ContactCard;
use carrychat_proto::sound;
use serde::Serialize;

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
        HeardCard::verified(ContactCard::from_bytes(&bytes).ok()?)
    }
}

/// `card` as one round of sound, a WAV file to play over and over.
pub(crate) fn wav(card: &str) -> crate::Result<Vec<u8>> {
    Ok(sound::wav(&ContactCard::from_text(card)?.to_bytes()))
}

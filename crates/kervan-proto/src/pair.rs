//! Keys shared by two contacts, and the tags derived from them (§4).

use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::PublicKey;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::identity::Identity;
use crate::{hmac_sha256, truncate, Error, Result};

pub const TAG_LEN: usize = 8;
pub const ACK_SECRET_LEN: usize = 16;

/// Days before today for which a receiver still recognises tags. Matches the
/// longest bundle lifetime.
pub const TAG_WINDOW_PAST_DAYS: u32 = 7;
/// Days after today for which a receiver recognises tags, for clock drift.
pub const TAG_WINDOW_FUTURE_DAYS: u32 = 1;

pub type RecipientTag = [u8; TAG_LEN];
pub type AckSecret = [u8; ACK_SECRET_LEN];

const PAIR_SALT: &[u8] = b"kervan/v0/pair";

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct PairKeys {
    enc: [u8; 32],
    tag: [u8; 32],
    ack: [u8; 32],
    root: [u8; 32],
    hdr: [u8; 32],
}

impl PairKeys {
    /// Derives the keys `me` shares with the contact whose X25519 key is
    /// `peer`. Both contacts derive the same keys.
    pub fn derive(me: &Identity, peer: &PublicKey) -> Result<Self> {
        let shared = me.dh_secret().diffie_hellman(peer);
        if !shared.was_contributory() {
            return Err(Error::WeakKey);
        }
        let own = me.dh_public();
        let (first, second) = if own.as_bytes() <= peer.as_bytes() {
            (own, *peer)
        } else {
            (*peer, own)
        };

        let hkdf = Hkdf::<Sha256>::new(Some(PAIR_SALT), shared.as_bytes());
        let expand = |label: &[u8]| {
            let mut key = [0; 32];
            hkdf.expand(
                &[label, first.as_bytes(), second.as_bytes()].concat(),
                &mut key,
            )
            .expect("32 bytes is a valid HKDF-SHA256 output length");
            key
        };
        Ok(Self {
            enc: expand(b"enc"),
            tag: expand(b"tag"),
            ack: expand(b"ack"),
            root: expand(b"root"),
            hdr: expand(b"hdr"),
        })
    }

    /// The tag of a bundle for `recipient` created on `day` (§4.1).
    pub fn recipient_tag(&self, recipient: &PublicKey, day: u32) -> RecipientTag {
        truncate(&hmac_sha256(
            &self.tag,
            &[recipient.as_bytes(), &day.to_be_bytes()],
        ))
    }

    /// The secret a recipient reveals to prove delivery (§5.1, §6).
    pub fn ack_secret(&self, nonce: &[u8; 24]) -> AckSecret {
        truncate(&hmac_sha256(&self.ack, &[nonce]))
    }

    pub(crate) fn enc_key(&self) -> &[u8; 32] {
        &self.enc
    }

    pub(crate) fn root_key(&self) -> &[u8; 32] {
        &self.root
    }

    pub(crate) fn header_key(&self) -> &[u8; 32] {
        &self.hdr
    }

    /// `K_enc`, `K_tag` and `K_ack`, for test vectors and debugging.
    pub fn expose_secret_keys(&self) -> (&[u8; 32], &[u8; 32], &[u8; 32]) {
        (&self.enc, &self.tag, &self.ack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_contacts_derive_the_same_keys() {
        let alice = Identity::generate("alice").unwrap();
        let bob = Identity::generate("bob").unwrap();
        let from_alice = PairKeys::derive(&alice, &bob.dh_public()).unwrap();
        let from_bob = PairKeys::derive(&bob, &alice.dh_public()).unwrap();
        assert_eq!(
            from_alice.expose_secret_keys(),
            from_bob.expose_secret_keys()
        );
    }

    #[test]
    fn tags_differ_per_direction_and_day() {
        let alice = Identity::generate("alice").unwrap();
        let bob = Identity::generate("bob").unwrap();
        let keys = PairKeys::derive(&alice, &bob.dh_public()).unwrap();
        let to_bob = keys.recipient_tag(&bob.dh_public(), 20_000);
        assert_ne!(to_bob, keys.recipient_tag(&alice.dh_public(), 20_000));
        assert_ne!(to_bob, keys.recipient_tag(&bob.dh_public(), 20_001));
    }

    #[test]
    fn low_order_peer_key_is_rejected() {
        let alice = Identity::generate("alice").unwrap();
        let identity_point = PublicKey::from([0; 32]);
        assert!(matches!(
            PairKeys::derive(&alice, &identity_point),
            Err(Error::WeakKey)
        ));
    }
}

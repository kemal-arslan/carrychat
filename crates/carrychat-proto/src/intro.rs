//! First contact (§5.5): a bundle for someone who does not have the sender's
//! card yet. It is sealed to the recipient's key with a one-time key, and
//! carries the sender's card and a signature, so the recipient can add the
//! sender after reading the first message.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::Signature;
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::bundle::{
    check_lifetime, Bundle, BundleId, Tombstone, AEAD_TAG_LEN, HEADER_LEN, MAX_BUNDLE_LEN,
};
use crate::identity::{ContactCard, Identity};
use crate::pair::{AckSecret, RecipientTag};
use crate::{hmac_sha256, sha256, truncate, Error, Result};

/// The plaintext kind of an introduction (§5.2).
pub const KIND_INTRODUCTION: u8 = 0x02;
const EPHEMERAL_LEN: usize = 32;
const SIGNATURE_LEN: usize = 64;
/// Kind, creation time and card length.
const FIXED_LEN: usize = 6;
const INTRO_SALT: &[u8] = b"carrychat/v0/intro";
const SIGNATURE_LABEL: &[u8] = b"carrychat/v0/intro-signature";
const MAILBOX_LABEL: &[u8] = b"carrychat/v0/mailbox";

/// Where introductions to `recipient` created on `day` are posted on links
/// that need a tag to find them, such as Nostr (§9.3). Anyone holding the
/// recipient's card can compute it.
pub fn mailbox_tag(recipient: &PublicKey, day: u32) -> RecipientTag {
    let key = sha256(&[MAILBOX_LABEL, recipient.as_bytes()]);
    truncate(&hmac_sha256(&key, &[&day.to_be_bytes()]))
}

/// An opened introduction: who wrote it and what they wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Introduction {
    pub card: ContactCard,
    pub created_at: u32,
    pub text: String,
    /// What the recipient publishes once it has the introduction (§6).
    pub tombstone: Tombstone,
}

#[derive(Zeroize, ZeroizeOnDrop)]
struct IntroKeys {
    enc: [u8; 32],
    ack: [u8; 32],
}

impl IntroKeys {
    fn derive(
        own: &StaticSecret,
        other: &PublicKey,
        ephemeral: &PublicKey,
        recipient: &PublicKey,
    ) -> Result<Self> {
        let shared = own.diffie_hellman(other);
        if !shared.was_contributory() {
            return Err(Error::WeakKey);
        }
        let hkdf = Hkdf::<Sha256>::new(Some(INTRO_SALT), shared.as_bytes());
        let expand = |label: &[u8]| {
            let mut key = [0; 32];
            hkdf.expand(
                &[label, ephemeral.as_bytes(), recipient.as_bytes()].concat(),
                &mut key,
            )
            .expect("32 bytes is a valid HKDF-SHA256 output length");
            key
        };
        Ok(Self {
            enc: expand(b"enc"),
            ack: expand(b"ack"),
        })
    }

    fn ack_secret(&self, nonce: &[u8; 24]) -> AckSecret {
        truncate(&hmac_sha256(&self.ack, &[nonce]))
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&self.enc).into())
    }
}

/// Seals a first message from `sender` to the holder of `recipient`, with a
/// fresh one-time key and nonce, under the recipient's mailbox tag for the
/// day it is created: whoever carries it to the internet relays posts it
/// where the recipient looks (§9.3).
pub fn seal_introduction(
    sender: &Identity,
    recipient: &PublicKey,
    text: &str,
    created_at: u32,
    expires_at: u32,
    hop_limit: u8,
) -> Result<Bundle> {
    let mut ephemeral = [0; 32];
    let mut nonce = [0; 24];
    OsRng.fill_bytes(&mut ephemeral);
    OsRng.fill_bytes(&mut nonce);
    let tag = mailbox_tag(recipient, crate::day_of(created_at));
    let bundle = seal_introduction_with(
        sender, recipient, text, created_at, expires_at, hop_limit, ephemeral, nonce, tag,
    );
    ephemeral.zeroize();
    bundle
}

/// Seals with caller-chosen randomness, for test vectors. Use
/// [`seal_introduction`].
#[allow(clippy::too_many_arguments)]
pub fn seal_introduction_with(
    sender: &Identity,
    recipient: &PublicKey,
    text: &str,
    created_at: u32,
    expires_at: u32,
    hop_limit: u8,
    ephemeral: [u8; 32],
    nonce: [u8; 24],
    tag: RecipientTag,
) -> Result<Bundle> {
    check_lifetime(created_at, expires_at)?;
    let ephemeral = StaticSecret::from(ephemeral);
    let ephemeral_public = PublicKey::from(&ephemeral);
    let keys = IntroKeys::derive(&ephemeral, recipient, &ephemeral_public, recipient)?;

    let card = sender.card().to_bytes();
    let signature = sender.sign(&transcript(
        &ephemeral_public,
        recipient,
        &nonce,
        created_at,
        text.as_bytes(),
    ));
    let mut plaintext = Vec::with_capacity(FIXED_LEN + card.len() + SIGNATURE_LEN + text.len());
    plaintext.push(KIND_INTRODUCTION);
    plaintext.extend_from_slice(&created_at.to_be_bytes());
    plaintext.push(card.len() as u8);
    plaintext.extend_from_slice(&card);
    plaintext.extend_from_slice(&signature.to_bytes());
    plaintext.extend_from_slice(text.as_bytes());
    if HEADER_LEN + EPHEMERAL_LEN + plaintext.len() + AEAD_TAG_LEN > MAX_BUNDLE_LEN {
        return Err(Error::TooLong);
    }

    let mut bundle = Bundle {
        hop_limit,
        expires_at,
        id: BundleId::from_ack_secret(&keys.ack_secret(&nonce)),
        tag,
        nonce,
        ciphertext: Vec::new(),
    };
    let sealed = keys
        .cipher()
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plaintext,
                aad: &bundle.aad(),
            },
        )
        .map_err(|_| Error::Invalid("plaintext"))?;
    bundle.ciphertext = [ephemeral_public.as_bytes().as_slice(), &sealed].concat();
    Ok(bundle)
}

/// Opens a bundle as an introduction to `me`. Fails for every bundle that is
/// not one, so a node can try this on bundles no contact's tag matched.
pub fn open_introduction(me: &Identity, bundle: &Bundle) -> Result<Introduction> {
    let (ephemeral, sealed) = bundle
        .ciphertext
        .split_at_checked(EPHEMERAL_LEN)
        .ok_or(Error::Truncated)?;
    let ephemeral = PublicKey::from(truncate::<32>(ephemeral));
    let own = me.dh_public();
    let keys = IntroKeys::derive(me.dh_secret(), &ephemeral, &ephemeral, &own)?;
    let plaintext = keys
        .cipher()
        .decrypt(
            XNonce::from_slice(&bundle.nonce),
            Payload {
                msg: sealed,
                aad: &bundle.aad(),
            },
        )
        .map_err(|_| Error::Decrypt)?;

    if plaintext.len() < FIXED_LEN || plaintext[0] != KIND_INTRODUCTION {
        return Err(Error::Invalid("introduction"));
    }
    let created_at = u32::from_be_bytes(truncate(&plaintext[1..5]));
    let card_end = FIXED_LEN + plaintext[5] as usize;
    let signature_end = card_end + SIGNATURE_LEN;
    if plaintext.len() < signature_end {
        return Err(Error::Truncated);
    }
    let card = ContactCard::from_bytes(&plaintext[FIXED_LEN..card_end])?;
    let signature = Signature::from_bytes(&truncate(&plaintext[card_end..signature_end]));
    let text = &plaintext[signature_end..];

    // The card proves who it describes; the signature proves the sender holds
    // that card's key, and binds it to this recipient and this bundle.
    card.signing_key()
        .verify_strict(
            &transcript(&ephemeral, &own, &bundle.nonce, created_at, text),
            &signature,
        )
        .map_err(|_| Error::BadSignature)?;
    check_lifetime(created_at, bundle.expires_at)?;
    let ack_secret = keys.ack_secret(&bundle.nonce);
    if BundleId::from_ack_secret(&ack_secret) != bundle.id {
        return Err(Error::Invalid("bundle id"));
    }
    Ok(Introduction {
        card,
        created_at,
        text: String::from_utf8(text.to_vec()).map_err(|_| Error::Invalid("text"))?,
        tombstone: Tombstone {
            id: bundle.id,
            ack_secret,
        },
    })
}

fn transcript(
    ephemeral: &PublicKey,
    recipient: &PublicKey,
    nonce: &[u8; 24],
    created_at: u32,
    text: &[u8],
) -> Vec<u8> {
    [
        SIGNATURE_LABEL,
        ephemeral.as_bytes(),
        recipient.as_bytes(),
        nonce,
        &created_at.to_be_bytes(),
        text,
    ]
    .concat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{seal, Message, DEFAULT_TTL_SECS};
    use crate::frame::{frames_for, MESHTASTIC_MTU};
    use crate::pair::PairKeys;

    const NOW: u32 = 1_790_000_000;

    fn introduce(sender: &Identity, recipient: &Identity, text: &str) -> Bundle {
        seal_introduction(
            sender,
            &recipient.dh_public(),
            text,
            NOW,
            NOW + DEFAULT_TTL_SECS,
            8,
        )
        .unwrap()
    }

    #[test]
    fn recipient_learns_the_sender_and_the_message() {
        let alice = Identity::generate("alice").unwrap();
        let bob = Identity::generate("bob").unwrap();
        let bundle = Bundle::from_bytes(&introduce(&alice, &bob, "Ben Alice").to_bytes()).unwrap();
        let intro = open_introduction(&bob, &bundle).unwrap();
        assert_eq!(intro.card, alice.card());
        assert_eq!(intro.text, "Ben Alice");
        assert!(intro.tombstone.verify());
        assert_eq!(intro.tombstone.id, bundle.id);
    }

    #[test]
    fn it_carries_the_recipients_mailbox_tag() {
        // So a node that carries it to the internet posts it where the
        // recipient looks, without knowing who that is.
        let alice = Identity::generate("alice").unwrap();
        let bob = Identity::generate("bob").unwrap();
        let bundle = introduce(&alice, &bob, "hi");
        assert_eq!(
            bundle.tag,
            mailbox_tag(&bob.dh_public(), crate::day_of(NOW))
        );
    }

    #[test]
    fn nobody_else_can_open_it() {
        let alice = Identity::generate("alice").unwrap();
        let bob = Identity::generate("bob").unwrap();
        let eve = Identity::generate("eve").unwrap();
        let bundle = introduce(&alice, &bob, "hi");
        assert_eq!(open_introduction(&eve, &bundle), Err(Error::Decrypt));
    }

    #[test]
    fn a_card_without_its_key_cannot_introduce_anyone() {
        // Eve holds Alice's card but not her key, and signs with her own.
        let alice = Identity::generate("alice").unwrap();
        let bob = Identity::generate("bob").unwrap();
        let eve = Identity::generate("eve").unwrap();
        let bundle = introduce(&eve, &bob, "Ben Alice");
        let intro = open_introduction(&bob, &bundle).unwrap();
        assert_eq!(intro.card, eve.card());

        let ephemeral = StaticSecret::from([9; 32]);
        let ephemeral_public = PublicKey::from(&ephemeral);
        let keys = IntroKeys::derive(
            &ephemeral,
            &bob.dh_public(),
            &ephemeral_public,
            &bob.dh_public(),
        )
        .unwrap();
        let card = alice.card().to_bytes();
        let nonce = [3; 24];
        let signature = eve.sign(&transcript(
            &ephemeral_public,
            &bob.dh_public(),
            &nonce,
            NOW,
            b"Ben Alice",
        ));
        let plaintext = [
            &[KIND_INTRODUCTION][..],
            &NOW.to_be_bytes(),
            &[card.len() as u8],
            &card,
            &signature.to_bytes(),
            b"Ben Alice",
        ]
        .concat();
        let mut forged = Bundle {
            hop_limit: 8,
            expires_at: NOW + 60,
            id: BundleId::from_ack_secret(&keys.ack_secret(&nonce)),
            tag: [0; 8],
            nonce,
            ciphertext: Vec::new(),
        };
        let sealed = keys
            .cipher()
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &forged.aad(),
                },
            )
            .unwrap();
        forged.ciphertext = [ephemeral_public.as_bytes().as_slice(), &sealed].concat();
        assert_eq!(open_introduction(&bob, &forged), Err(Error::BadSignature));
    }

    #[test]
    fn a_normal_bundle_is_not_an_introduction() {
        let alice = Identity::generate("alice").unwrap();
        let bob = Identity::generate("bob").unwrap();
        let keys = PairKeys::derive(&alice, &bob.dh_public()).unwrap();
        let bundle = seal(
            &keys,
            &bob.dh_public(),
            &Message::text("hi", NOW),
            NOW + 60,
            8,
        )
        .unwrap();
        assert!(open_introduction(&bob, &bundle).is_err());
    }

    #[test]
    fn a_short_introduction_fits_two_meshtastic_packets() {
        let alice = Identity::generate("alice").unwrap();
        let bob = Identity::generate("bob").unwrap();
        let bundle = introduce(&alice, &bob, &"a".repeat(140));
        assert_eq!(frames_for(&bundle, MESHTASTIC_MTU).unwrap().len(), 2);
    }
}

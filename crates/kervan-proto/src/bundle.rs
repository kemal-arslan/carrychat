//! Sealed messages and delivery proofs (§5, §6).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand_core::{OsRng, RngCore};
use x25519_dalek::PublicKey;

use crate::identity::ContactCard;
use crate::pair::{AckSecret, PairKeys, RecipientTag, TAG_LEN};
use crate::{day_of, sha256, truncate, Error, Result, DAY_SECS, VERSION};

pub const HEADER_LEN: usize = 46;
pub const MESSAGE_HEADER_LEN: usize = 5;
pub const AEAD_TAG_LEN: usize = 16;
/// Bytes a bundle adds around a message body (§5.4).
pub const OVERHEAD: usize = HEADER_LEN + MESSAGE_HEADER_LEN + AEAD_TAG_LEN;
pub const MAX_BUNDLE_LEN: usize = 4096;
pub const DEFAULT_TTL_SECS: u32 = 3 * DAY_SECS;
pub const MAX_TTL_SECS: u32 = 7 * DAY_SECS;
pub const DEFAULT_HOP_LIMIT: u8 = 8;

const BUNDLE_ID_LABEL: &[u8] = b"kervan/v0/ack";
const KIND_TEXT: u8 = 0x01;
const KIND_READ: u8 = 0x03;
const KIND_CARD: u8 = 0x04;
/// Bundle ids in one read receipt, at most.
pub const MAX_READ_IDS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct BundleId(pub [u8; 8]);

impl BundleId {
    pub fn from_ack_secret(secret: &AckSecret) -> Self {
        Self(truncate(&sha256(&[BUNDLE_ID_LABEL, secret])))
    }
}

/// One sealed message: the unit that is stored, carried and forwarded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    /// The only field relays may change. It is not authenticated.
    pub hop_limit: u8,
    pub expires_at: u32,
    pub id: BundleId,
    pub tag: RecipientTag,
    pub nonce: [u8; 24],
    pub ciphertext: Vec<u8>,
}

impl Bundle {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + self.ciphertext.len());
        bytes.push(VERSION);
        bytes.push(self.hop_limit);
        bytes.extend_from_slice(&self.aad()[1..]);
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }

    /// Parses the bundle structure. Only the recipient can check the rest.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + MESSAGE_HEADER_LEN + AEAD_TAG_LEN {
            return Err(Error::Truncated);
        }
        if bytes.len() > MAX_BUNDLE_LEN {
            return Err(Error::TooLong);
        }
        if bytes[0] != VERSION {
            return Err(Error::UnsupportedVersion(bytes[0]));
        }
        Ok(Self {
            hop_limit: bytes[1],
            expires_at: u32::from_be_bytes(truncate(&bytes[2..6])),
            id: BundleId(truncate(&bytes[6..14])),
            tag: truncate(&bytes[14..14 + TAG_LEN]),
            nonce: truncate(&bytes[22..46]),
            ciphertext: bytes[HEADER_LEN..].to_vec(),
        })
    }

    pub fn is_expired(&self, now: u32) -> bool {
        now >= self.expires_at
    }

    /// The header without the hop limit (§5.1).
    pub(crate) fn aad(&self) -> [u8; HEADER_LEN - 1] {
        let mut aad = [0; HEADER_LEN - 1];
        aad[0] = VERSION;
        aad[1..5].copy_from_slice(&self.expires_at.to_be_bytes());
        aad[5..13].copy_from_slice(&self.id.0);
        aad[13..21].copy_from_slice(&self.tag);
        aad[21..45].copy_from_slice(&self.nonce);
        aad
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Text,
    /// Ids of the sender's bundles that the recipient has read.
    Read,
    /// The sender's contact card, after they changed their name.
    Card,
    /// A kind this version does not know. The body is kept but not shown.
    Unknown(u8),
}

impl Kind {
    fn to_byte(self) -> u8 {
        match self {
            Kind::Text => KIND_TEXT,
            Kind::Read => KIND_READ,
            Kind::Card => KIND_CARD,
            Kind::Unknown(byte) => byte,
        }
    }

    fn from_byte(byte: u8) -> Self {
        match byte {
            KIND_TEXT => Kind::Text,
            KIND_READ => Kind::Read,
            KIND_CARD => Kind::Card,
            other => Kind::Unknown(other),
        }
    }
}

/// The plaintext inside a bundle (§5.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Message {
    pub kind: Kind,
    pub created_at: u32,
    pub body: Vec<u8>,
}

impl Message {
    pub fn text(text: &str, created_at: u32) -> Self {
        Self {
            kind: Kind::Text,
            created_at,
            body: text.as_bytes().to_vec(),
        }
    }

    /// A read receipt for up to [`MAX_READ_IDS`] of the recipient's bundles.
    pub fn read(ids: &[BundleId], created_at: u32) -> Self {
        Self {
            kind: Kind::Read,
            created_at,
            body: ids.iter().take(MAX_READ_IDS).flat_map(|id| id.0).collect(),
        }
    }

    /// The bundle ids in a read receipt. Trailing bytes are ignored.
    pub fn read_ids(&self) -> Vec<BundleId> {
        self.body
            .chunks_exact(8)
            .take(MAX_READ_IDS)
            .map(|id| BundleId(truncate(id)))
            .collect()
    }

    /// The sender's new contact card.
    pub fn card(card: &ContactCard, created_at: u32) -> Self {
        Self {
            kind: Kind::Card,
            created_at,
            body: card.to_bytes(),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MESSAGE_HEADER_LEN + self.body.len());
        bytes.push(self.kind.to_byte());
        bytes.extend_from_slice(&self.created_at.to_be_bytes());
        bytes.extend_from_slice(&self.body);
        bytes
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < MESSAGE_HEADER_LEN {
            return Err(Error::Truncated);
        }
        Ok(Self {
            kind: Kind::from_byte(bytes[0]),
            created_at: u32::from_be_bytes(truncate(&bytes[1..5])),
            body: bytes[MESSAGE_HEADER_LEN..].to_vec(),
        })
    }
}

/// Seals `message` for `recipient` with a random nonce.
pub fn seal(
    keys: &PairKeys,
    recipient: &PublicKey,
    message: &Message,
    expires_at: u32,
    hop_limit: u8,
) -> Result<Bundle> {
    let mut nonce = [0; 24];
    OsRng.fill_bytes(&mut nonce);
    seal_with_nonce(keys, recipient, message, expires_at, hop_limit, nonce)
}

/// Seals with a caller-chosen nonce, for test vectors. Reusing a nonce with the
/// same pair keys breaks confidentiality: use [`seal`].
pub fn seal_with_nonce(
    keys: &PairKeys,
    recipient: &PublicKey,
    message: &Message,
    expires_at: u32,
    hop_limit: u8,
    nonce: [u8; 24],
) -> Result<Bundle> {
    check_lifetime(message.created_at, expires_at)?;
    let plaintext = message.to_bytes();
    if HEADER_LEN + plaintext.len() + AEAD_TAG_LEN > MAX_BUNDLE_LEN {
        return Err(Error::TooLong);
    }

    let mut bundle = Bundle {
        hop_limit,
        expires_at,
        id: BundleId::from_ack_secret(&keys.ack_secret(&nonce)),
        tag: keys.recipient_tag(recipient, day_of(message.created_at)),
        nonce,
        ciphertext: Vec::new(),
    };
    bundle.ciphertext = cipher(keys)
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plaintext,
                aad: &bundle.aad(),
            },
        )
        .map_err(|_| Error::Invalid("plaintext"))?;
    Ok(bundle)
}

/// Opens a bundle addressed to `me` with the keys of the contact its tag
/// pointed to (§5.3).
pub fn open(keys: &PairKeys, me: &PublicKey, bundle: &Bundle) -> Result<Message> {
    let plaintext = cipher(keys)
        .decrypt(
            XNonce::from_slice(&bundle.nonce),
            Payload {
                msg: &bundle.ciphertext,
                aad: &bundle.aad(),
            },
        )
        .map_err(|_| Error::Decrypt)?;
    let message = Message::from_bytes(&plaintext)?;

    // The header is authenticated, but a faulty sender could still have
    // written fields that disagree with the content.
    if keys.recipient_tag(me, day_of(message.created_at)) != bundle.tag {
        return Err(Error::Invalid("recipient tag"));
    }
    check_lifetime(message.created_at, bundle.expires_at)?;
    if BundleId::from_ack_secret(&keys.ack_secret(&bundle.nonce)) != bundle.id {
        return Err(Error::Invalid("bundle id"));
    }
    Ok(message)
}

/// Proof that a bundle was delivered (§6).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tombstone {
    pub id: BundleId,
    pub ack_secret: AckSecret,
}

impl Tombstone {
    /// Only the two contacts of a pair can make the tombstone of their bundle.
    pub fn for_bundle(keys: &PairKeys, bundle: &Bundle) -> Self {
        Self {
            id: bundle.id,
            ack_secret: keys.ack_secret(&bundle.nonce),
        }
    }

    /// Whether the secret belongs to the id. Act on a tombstone only for a
    /// bundle you hold or have seen: anyone can make one for an id they made up.
    pub fn verify(&self) -> bool {
        BundleId::from_ack_secret(&self.ack_secret) == self.id
    }
}

fn cipher(keys: &PairKeys) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(keys.enc_key().into())
}

pub(crate) fn check_lifetime(created_at: u32, expires_at: u32) -> Result<()> {
    if expires_at <= created_at || expires_at - created_at > MAX_TTL_SECS {
        return Err(Error::Invalid("expiry"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::MESHTASTIC_MTU;
    use crate::identity::Identity;

    const NOW: u32 = 1_790_000_000;

    struct Pair {
        alice: Identity,
        bob: Identity,
        keys: PairKeys,
    }

    fn pair() -> Pair {
        let alice = Identity::generate("alice").unwrap();
        let bob = Identity::generate("bob").unwrap();
        let keys = PairKeys::derive(&alice, &bob.dh_public()).unwrap();
        Pair { alice, bob, keys }
    }

    fn seal_text(pair: &Pair, text: &str) -> Bundle {
        seal(
            &pair.keys,
            &pair.bob.dh_public(),
            &Message::text(text, NOW),
            NOW + DEFAULT_TTL_SECS,
            DEFAULT_HOP_LIMIT,
        )
        .unwrap()
    }

    #[test]
    fn recipient_opens_what_sender_sealed() {
        let pair = pair();
        let bundle =
            Bundle::from_bytes(&seal_text(&pair, "İyiyim, Kadıköy'deyim.").to_bytes()).unwrap();
        let bob_keys = PairKeys::derive(&pair.bob, &pair.alice.dh_public()).unwrap();
        let message = open(&bob_keys, &pair.bob.dh_public(), &bundle).unwrap();
        assert_eq!(message, Message::text("İyiyim, Kadıköy'deyim.", NOW));
    }

    #[test]
    fn relays_may_change_the_hop_limit() {
        let pair = pair();
        let mut bundle = seal_text(&pair, "hi");
        bundle.hop_limit -= 1;
        assert!(open(&pair.keys, &pair.bob.dh_public(), &bundle).is_ok());
    }

    #[test]
    fn any_other_change_breaks_the_bundle() {
        let pair = pair();
        let original = seal_text(&pair, "hi").to_bytes();
        for index in (0..original.len()).filter(|&i| i != 1) {
            let mut bytes = original.clone();
            bytes[index] ^= 0x01;
            let Ok(bundle) = Bundle::from_bytes(&bytes) else {
                continue;
            };
            assert_eq!(
                open(&pair.keys, &pair.bob.dh_public(), &bundle),
                Err(Error::Decrypt),
                "byte {index}"
            );
        }
    }

    #[test]
    fn strangers_cannot_open_a_bundle() {
        let pair = pair();
        let bundle = seal_text(&pair, "hi");
        let eve = Identity::generate("eve").unwrap();
        let eve_keys = PairKeys::derive(&eve, &pair.alice.dh_public()).unwrap();
        assert_eq!(
            open(&eve_keys, &eve.dh_public(), &bundle),
            Err(Error::Decrypt)
        );
    }

    #[test]
    fn tombstone_proves_delivery() {
        let pair = pair();
        let bundle = seal_text(&pair, "hi");
        let bob_keys = PairKeys::derive(&pair.bob, &pair.alice.dh_public()).unwrap();
        let tombstone = Tombstone::for_bundle(&bob_keys, &bundle);
        assert!(tombstone.verify());
        assert_eq!(tombstone.id, bundle.id);

        let forged = Tombstone {
            id: bundle.id,
            ack_secret: [0x42; 16],
        };
        assert!(!forged.verify());
    }

    #[test]
    fn lifetime_is_bounded() {
        let pair = pair();
        let message = Message::text("hi", NOW);
        for expires_at in [NOW, NOW - 1, NOW + MAX_TTL_SECS + 1] {
            assert_eq!(
                seal(&pair.keys, &pair.bob.dh_public(), &message, expires_at, 8),
                Err(Error::Invalid("expiry"))
            );
        }
    }

    #[test]
    fn oversized_messages_are_refused() {
        let pair = pair();
        let body = "a".repeat(MAX_BUNDLE_LEN - OVERHEAD + 1);
        assert_eq!(
            seal(
                &pair.keys,
                &pair.bob.dh_public(),
                &Message::text(&body, NOW),
                NOW + 60,
                8
            ),
            Err(Error::TooLong)
        );
    }

    #[test]
    fn a_165_byte_message_fits_one_meshtastic_packet() {
        let pair = pair();
        let fits = seal_text(&pair, &"a".repeat(165));
        assert_eq!(1 + fits.to_bytes().len(), MESHTASTIC_MTU);
    }
}

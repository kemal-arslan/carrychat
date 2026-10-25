//! Public channels (§5.8): conversations that anyone who knows the name, and
//! the password if there is one, can read and write. Every post is signed
//! with a key its author uses for that channel only.

use core::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand_core::CryptoRngCore;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::bundle::{
    Bundle, BundleId, Kind, Message, AEAD_TAG_LEN, HEADER_LEN as BUNDLE_HEADER_LEN, MAX_BUNDLE_LEN,
    MESSAGE_HEADER_LEN,
};
use crate::pair::RecipientTag;
use crate::{day_of, hmac_sha256, sha256, truncate, Error, Result, DAY_SECS};

pub const MAX_CHANNEL_NAME_LEN: usize = 32;
pub const MAX_NICK_LEN: usize = 20;
/// The longest a channel bundle may live.
pub const CHANNEL_TTL_SECS: u32 = DAY_SECS;
pub const CHANNEL_HOP_LIMIT: u8 = 4;
pub const SIGNATURE_LEN: usize = 64;
/// Bytes a channel bundle adds around a post's body, besides the nickname.
pub const OVERHEAD: usize =
    BUNDLE_HEADER_LEN + MESSAGE_HEADER_LEN + 32 + 1 + AEAD_TAG_LEN + SIGNATURE_LEN;

const SALT: &[u8] = b"kervan/v1/channel";
const BUNDLE_ID_LABEL: &[u8] = b"kervan/v1/channel";
const SIGNATURE_LABEL: &[u8] = b"kervan/v1/channel-signature";
const AUTHOR_LABEL: &[u8] = b"kervan/v1/channel-key";
const KEYS_LEN: usize = 96;

/// A channel name as everyone derives the keys from it: trimmed, without a
/// leading `#`, lowercase, of letters, digits, `-` and `_` (§5.8).
pub fn normalize_name(name: &str) -> Result<String> {
    let name = name.trim();
    // İ lowercases to i and a combining dot; Turkish writes it as i.
    let name = name
        .strip_prefix('#')
        .unwrap_or(name)
        .to_lowercase()
        .replace("i\u{307}", "i");
    if name.is_empty() || name.len() > MAX_CHANNEL_NAME_LEN {
        return Err(Error::Invalid("channel name length"));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(Error::Invalid("channel name"));
    }
    Ok(name)
}

/// What a reader learns from a channel bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post {
    /// The kind, the time and the body: text, or the author's card.
    pub message: Message,
    /// The author's key for this channel.
    pub author: [u8; 32],
    pub nick: String,
}

/// The keys of one channel, from its name and password.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct ChannelKeys {
    enc: [u8; 32],
    tag: [u8; 32],
    id: [u8; 32],
}

impl ChannelKeys {
    /// Derives the keys of a channel. The name is normalized first.
    pub fn derive(name: &str, password: &str) -> Result<Self> {
        let name = normalize_name(name)?;
        let ikm = [name.as_bytes(), &[0], password.as_bytes()].concat();
        let hkdf = Hkdf::<Sha256>::new(Some(SALT), &ikm);
        let mut keys = Self {
            enc: [0; 32],
            tag: [0; 32],
            id: [0; 32],
        };
        for (info, key) in [
            (&b"enc"[..], &mut keys.enc),
            (b"tag", &mut keys.tag),
            (b"id", &mut keys.id),
        ] {
            hkdf.expand(info, key)
                .expect("32 bytes is a valid HKDF-SHA256 output length");
        }
        Ok(keys)
    }

    pub fn tag(&self, day: u32) -> RecipientTag {
        truncate(&hmac_sha256(&self.tag, &[&day.to_be_bytes()]))
    }

    /// Names the channel without revealing its keys.
    pub fn id(&self) -> [u8; 32] {
        self.id
    }

    /// The seed of our signing key in this channel, from our own Ed25519
    /// secret, so it is the same every time but tells nobody who we are.
    pub fn author_seed(&self, signing_secret: &[u8; 32]) -> [u8; 32] {
        hmac_sha256(signing_secret, &[AUTHOR_LABEL, &self.id])
    }

    pub fn seal(
        &self,
        author_seed: &[u8; 32],
        nick: &str,
        message: &Message,
        expires_at: u32,
        hop_limit: u8,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Bundle> {
        let mut nonce = [0; 24];
        rng.fill_bytes(&mut nonce);
        self.seal_with_nonce(author_seed, nick, message, expires_at, hop_limit, nonce)
    }

    /// Seals with a caller-chosen nonce, for test vectors. Use [`ChannelKeys::seal`].
    pub fn seal_with_nonce(
        &self,
        author_seed: &[u8; 32],
        nick: &str,
        message: &Message,
        expires_at: u32,
        hop_limit: u8,
        nonce: [u8; 24],
    ) -> Result<Bundle> {
        if nick.len() > MAX_NICK_LEN {
            return Err(Error::Invalid("nickname length"));
        }
        if !matches!(message.kind, Kind::Text | Kind::Card | Kind::File) {
            return Err(Error::Invalid("kind of post"));
        }
        check_lifetime(message.created_at, expires_at)?;
        let signing = SigningKey::from_bytes(author_seed);
        let body = [
            signing.verifying_key().as_bytes().as_slice(),
            &[nick.len() as u8],
            nick.as_bytes(),
            &message.body,
        ]
        .concat();
        let plaintext = Message {
            kind: message.kind,
            created_at: message.created_at,
            body,
        }
        .to_bytes();
        if BUNDLE_HEADER_LEN + plaintext.len() + AEAD_TAG_LEN + SIGNATURE_LEN > MAX_BUNDLE_LEN {
            return Err(Error::TooLong);
        }

        let mut bundle = Bundle {
            hop_limit,
            expires_at,
            id: bundle_id(&nonce),
            tag: self.tag(day_of(message.created_at)),
            nonce,
            ciphertext: Vec::new(),
        };
        let aad = bundle.aad();
        let sealed = XChaCha20Poly1305::new((&self.enc).into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Invalid("plaintext"))?;
        let signature = signing.sign(&[SIGNATURE_LABEL, &aad, &sealed].concat());
        bundle.ciphertext = [sealed.as_slice(), &signature.to_bytes()].concat();
        Ok(bundle)
    }

    /// Opens a bundle whose tag is one of this channel's (§5.8).
    pub fn open(&self, bundle: &Bundle) -> Result<Post> {
        let length = bundle.ciphertext.len();
        if length < MESSAGE_HEADER_LEN + 33 + AEAD_TAG_LEN + SIGNATURE_LEN {
            return Err(Error::Invalid("not a channel post"));
        }
        let (sealed, signature) = bundle.ciphertext.split_at(length - SIGNATURE_LEN);
        let aad = bundle.aad();
        let plaintext = XChaCha20Poly1305::new((&self.enc).into())
            .decrypt(
                XNonce::from_slice(&bundle.nonce),
                Payload {
                    msg: sealed,
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Decrypt)?;
        let message = Message::from_bytes(&plaintext)?;
        if message.body.len() < 33 {
            return Err(Error::Truncated);
        }
        let author: [u8; 32] = truncate(&message.body);
        let nick_len = message.body[32] as usize;
        if nick_len > MAX_NICK_LEN || message.body.len() < 33 + nick_len {
            return Err(Error::Invalid("nickname length"));
        }
        let nick = String::from_utf8(message.body[33..33 + nick_len].to_vec())
            .map_err(|_| Error::Invalid("nickname"))?;
        VerifyingKey::from_bytes(&author)
            .map_err(|_| Error::Invalid("author key"))?
            .verify_strict(
                &[SIGNATURE_LABEL, &aad, sealed].concat(),
                &Signature::from_bytes(&truncate(signature)),
            )
            .map_err(|_| Error::BadSignature)?;
        if self.tag(day_of(message.created_at)) != bundle.tag {
            return Err(Error::Invalid("channel tag"));
        }
        check_lifetime(message.created_at, bundle.expires_at)?;
        if bundle_id(&bundle.nonce) != bundle.id {
            return Err(Error::Invalid("bundle id"));
        }
        Ok(Post {
            message: Message {
                kind: message.kind,
                created_at: message.created_at,
                body: message.body[33 + nick_len..].to_vec(),
            },
            author,
            nick,
        })
    }

    pub fn to_bytes(&self) -> [u8; KEYS_LEN] {
        let mut bytes = [0; KEYS_LEN];
        bytes[..32].copy_from_slice(&self.enc);
        bytes[32..64].copy_from_slice(&self.tag);
        bytes[64..].copy_from_slice(&self.id);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != KEYS_LEN {
            return Err(Error::Invalid("channel keys"));
        }
        Ok(Self {
            enc: truncate(bytes),
            tag: truncate(&bytes[32..]),
            id: truncate(&bytes[64..]),
        })
    }
}

impl fmt::Debug for ChannelKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChannelKeys").finish_non_exhaustive()
    }
}

/// The public key of an author seed, as readers see it.
pub fn author_key(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

fn check_lifetime(created_at: u32, expires_at: u32) -> Result<()> {
    if expires_at <= created_at || expires_at - created_at > CHANNEL_TTL_SECS {
        return Err(Error::Invalid("expiry"));
    }
    Ok(())
}

fn bundle_id(nonce: &[u8; 24]) -> BundleId {
    BundleId(truncate(&sha256(&[BUNDLE_ID_LABEL, nonce])))
}

#[cfg(test)]
mod tests {
    use rand_core::OsRng;

    use super::*;
    use crate::frame::{frames_for, MESHTASTIC_MTU};
    use crate::identity::Identity;

    const NOW: u32 = 1_790_000_000;

    fn post(keys: &ChannelKeys, seed: &[u8; 32], nick: &str, text: &str) -> Bundle {
        keys.seal(
            seed,
            nick,
            &Message::text(text, NOW),
            NOW + CHANNEL_TTL_SECS,
            CHANNEL_HOP_LIMIT,
            &mut OsRng,
        )
        .unwrap()
    }

    #[test]
    fn anyone_with_the_name_reads_and_sees_the_same_author() {
        let alice = Identity::generate("alice").unwrap();
        let keys = ChannelKeys::derive("#Mahalle", "").unwrap();
        let seed = keys.author_seed(&alice.secret_bytes().0);
        let bundle =
            Bundle::from_bytes(&post(&keys, &seed, "Ayşe", "Su geldi").to_bytes()).unwrap();

        let reader = ChannelKeys::derive("mahalle", "").unwrap();
        let read = reader.open(&bundle).unwrap();
        assert_eq!(read.message, Message::text("Su geldi", NOW));
        assert_eq!(read.nick, "Ayşe");
        assert_eq!(read.author, author_key(&seed));
        assert_eq!(bundle.tag, reader.tag(day_of(NOW)));
    }

    #[test]
    fn the_wrong_name_or_password_reads_nothing() {
        let keys = ChannelKeys::derive("aile", "gizli").unwrap();
        let bundle = post(&keys, &[1; 32], "x", "selam");
        for (name, password) in [("aile", ""), ("aile", "Gizli"), ("aile2", "gizli")] {
            let other = ChannelKeys::derive(name, password).unwrap();
            assert_ne!(other.tag(day_of(NOW)), bundle.tag);
            assert!(other.open(&bundle).is_err());
        }
    }

    #[test]
    fn authors_differ_by_channel_and_cannot_be_forged() {
        let alice = Identity::generate("alice").unwrap();
        let (mahalle, aile) = (
            ChannelKeys::derive("mahalle", "").unwrap(),
            ChannelKeys::derive("aile", "").unwrap(),
        );
        let secret = alice.secret_bytes().0;
        assert_ne!(
            author_key(&mahalle.author_seed(&secret)),
            author_key(&aile.author_seed(&secret))
        );

        // Signed by someone else but claiming Alice's key.
        let seed = mahalle.author_seed(&secret);
        let mut forged = post(&mahalle, &[9; 32], "alice", "ben Alice'im");
        let plaintext = XChaCha20Poly1305::new((&mahalle.enc).into())
            .decrypt(
                XNonce::from_slice(&forged.nonce),
                Payload {
                    msg: &forged.ciphertext[..forged.ciphertext.len() - SIGNATURE_LEN],
                    aad: &forged.aad(),
                },
            )
            .unwrap();
        let mut claimed = plaintext.clone();
        claimed[MESSAGE_HEADER_LEN..MESSAGE_HEADER_LEN + 32].copy_from_slice(&author_key(&seed));
        let sealed = XChaCha20Poly1305::new((&mahalle.enc).into())
            .encrypt(
                XNonce::from_slice(&forged.nonce),
                Payload {
                    msg: &claimed,
                    aad: &forged.aad(),
                },
            )
            .unwrap();
        let signature = forged.ciphertext[forged.ciphertext.len() - SIGNATURE_LEN..].to_vec();
        forged.ciphertext = [sealed, signature].concat();
        assert_eq!(mahalle.open(&forged).unwrap_err(), Error::BadSignature);
    }

    #[test]
    fn any_changed_byte_is_refused() {
        let keys = ChannelKeys::derive("mahalle", "").unwrap();
        let original = post(&keys, &[3; 32], "deniz", "hi").to_bytes();
        for index in (0..original.len()).filter(|&i| i != 1) {
            let mut bytes = original.clone();
            bytes[index] ^= 0x01;
            let Ok(bundle) = Bundle::from_bytes(&bytes) else {
                continue;
            };
            assert!(keys.open(&bundle).is_err(), "byte {index}");
        }
    }

    #[test]
    fn posts_live_a_day_at_most() {
        let keys = ChannelKeys::derive("mahalle", "").unwrap();
        let message = Message::text("uzun", NOW);
        assert!(keys
            .seal(
                &[1; 32],
                "x",
                &message,
                NOW + CHANNEL_TTL_SECS + 1,
                4,
                &mut OsRng
            )
            .is_err());
        assert!(keys
            .seal(
                &[1; 32],
                &"x".repeat(MAX_NICK_LEN + 1),
                &message,
                NOW + 60,
                4,
                &mut OsRng
            )
            .is_err());
        assert!(keys
            .seal(
                &[1; 32],
                "x",
                &Message::session(NOW),
                NOW + 60,
                4,
                &mut OsRng
            )
            .is_err());
    }

    #[test]
    fn names_are_normalized() {
        assert_eq!(normalize_name("  #Mahalle ").unwrap(), "mahalle");
        assert_eq!(normalize_name("kadıköy-su_2").unwrap(), "kadıköy-su_2");
        assert_eq!(normalize_name("#İzmir").unwrap(), "izmir");
        for bad in ["", "#", "iki kelime", "a/b", &"a".repeat(33)] {
            assert!(normalize_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            ChannelKeys::derive("#Mahalle", "").unwrap(),
            ChannelKeys::derive("mahalle", "").unwrap()
        );
    }

    #[test]
    fn keys_survive_a_round_trip() {
        let keys = ChannelKeys::derive("mahalle", "şifre").unwrap();
        assert_eq!(ChannelKeys::from_bytes(&keys.to_bytes()).unwrap(), keys);
    }

    #[test]
    fn a_short_post_fits_one_meshtastic_packet() {
        let keys = ChannelKeys::derive("mahalle", "").unwrap();
        let text = "a".repeat(MESHTASTIC_MTU - 1 - OVERHEAD - 8);
        assert_eq!(text.len(), 60);
        let bundle = post(&keys, &[5; 32], "8 bayt..", &text);
        assert_eq!(frames_for(&bundle, MESHTASTIC_MTU).unwrap().len(), 1);
    }
}

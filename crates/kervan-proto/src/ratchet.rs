//! Ratchet sessions (§5.6): the Double Ratchet over the pair keys, with an
//! encrypted header, so a key taken from a device later opens neither what
//! was recorded before nor, after a reply each way, what comes after.
//!
//! Nothing here stores anything: [`Session::open`] works on a copy and
//! returns the new state with the keys to keep, for the caller to commit only
//! when it accepts the message.

use core::fmt;

use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::XChaCha20;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rand_core::CryptoRngCore;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::bundle::{
    check_lifetime, check_opened, Bundle, BundleId, Message, AEAD_TAG_LEN,
    HEADER_LEN as BUNDLE_HEADER_LEN, MAX_BUNDLE_LEN, MESSAGE_HEADER_LEN,
};
use crate::identity::Identity;
use crate::pair::PairKeys;
use crate::{day_of, hmac_sha256, truncate, Error, Result};

/// Message keys a receiver derives ahead for messages that have not arrived.
pub const MAX_SKIP: u16 = 1000;
/// Mode byte, sender's ratchet key, previous chain length, message number.
pub const HEADER_LEN: usize = 37;
/// Bytes a ratchet bundle adds around a message body.
pub const OVERHEAD: usize = BUNDLE_HEADER_LEN + HEADER_LEN + MESSAGE_HEADER_LEN + AEAD_TAG_LEN;
const MODE_RATCHET: u8 = 0x01;
const RATCHET_INFO: &[u8] = b"kervan/v1/ratchet";
const STATE_LEN: usize = 170;

/// Who starts the session: the contact whose X25519 key sorts first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Initiator,
    Responder,
}

impl Role {
    pub fn of(own: &PublicKey, peer: &PublicKey) -> Self {
        if own.as_bytes() < peer.as_bytes() {
            Role::Initiator
        } else {
            Role::Responder
        }
    }
}

/// One side of a ratchet session with one contact.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct Session {
    #[zeroize(skip)]
    role: Role,
    /// Own ratchet secret key.
    own: [u8; 32],
    /// The peer's current ratchet public key.
    peer: Option<[u8; 32]>,
    root: [u8; 32],
    send: Option<[u8; 32]>,
    recv: Option<[u8; 32]>,
    sent: u16,
    received: u16,
    previous: u16,
}

/// The key of a message that has not arrived yet.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct SkippedKey {
    pub ratchet: [u8; 32],
    pub n: u16,
    pub key: [u8; 32],
}

pub struct Opened {
    pub message: Message,
    /// The state to keep if the message is accepted.
    pub session: Session,
    /// Keys of messages that have not arrived yet, to store.
    pub skipped: Vec<SkippedKey>,
    /// The stored skipped key this message used, to delete.
    pub used: Option<([u8; 32], u16)>,
}

impl Session {
    /// The state both contacts start from (§5.6).
    pub fn new(me: &Identity, peer: &PublicKey, keys: &PairKeys) -> Self {
        let role = Role::of(&me.dh_public(), peer);
        Self {
            role,
            own: me.dh_secret().to_bytes(),
            peer: (role == Role::Initiator).then(|| peer.to_bytes()),
            root: *keys.root_key(),
            send: None,
            recv: None,
            sent: 0,
            received: 0,
            previous: 0,
        }
    }

    pub fn role(&self) -> Role {
        self.role
    }

    /// Whether this side can send ratchet messages now.
    pub fn can_send(&self) -> bool {
        self.send.is_some() || (self.role == Role::Initiator && self.peer.is_some())
    }

    /// Whether this side has a sending chain: for the initiator, whether it
    /// has started the session.
    pub fn started(&self) -> bool {
        self.send.is_some()
    }

    /// Whether ratchet messages have gone both ways.
    pub fn is_established(&self) -> bool {
        self.send.is_some() && self.recv.is_some()
    }

    pub fn seal(
        &mut self,
        keys: &PairKeys,
        recipient: &PublicKey,
        message: &Message,
        expires_at: u32,
        hop_limit: u8,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Bundle> {
        let mut nonce = [0; 24];
        rng.fill_bytes(&mut nonce);
        self.seal_with_nonce(keys, recipient, message, expires_at, hop_limit, nonce, rng)
    }

    /// Seals with a caller-chosen nonce, for test vectors. Use [`Session::seal`].
    #[allow(clippy::too_many_arguments)]
    pub fn seal_with_nonce(
        &mut self,
        keys: &PairKeys,
        recipient: &PublicKey,
        message: &Message,
        expires_at: u32,
        hop_limit: u8,
        nonce: [u8; 24],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Bundle> {
        check_lifetime(message.created_at, expires_at)?;
        let plaintext = message.to_bytes();
        if BUNDLE_HEADER_LEN + HEADER_LEN + plaintext.len() + AEAD_TAG_LEN > MAX_BUNDLE_LEN {
            return Err(Error::TooLong);
        }

        let mut next = self.clone();
        if next.send.is_none() {
            // Only the initiator starts a session.
            let peer = match (next.role, next.peer) {
                (Role::Initiator, Some(peer)) => peer,
                _ => return Err(Error::NoSession),
            };
            let fresh = random_secret(rng);
            let (root, chain) = kdf_rk(&next.root, &dh(&fresh, &peer)?);
            next.own = fresh;
            next.root = root;
            next.send = Some(chain);
        }
        if next.sent == u16::MAX {
            return Err(Error::NoSession);
        }
        let (chain, key) = kdf_ck(&next.send.expect("a sending chain exists"));
        next.send = Some(chain);
        let mut header = [0; HEADER_LEN];
        header[0] = MODE_RATCHET;
        header[1..33].copy_from_slice(&public(&next.own));
        header[33..35].copy_from_slice(&next.previous.to_be_bytes());
        header[35..37].copy_from_slice(&next.sent.to_be_bytes());
        next.sent += 1;

        let mut bundle = Bundle {
            hop_limit,
            expires_at,
            id: BundleId::from_ack_secret(&keys.ack_secret(&nonce)),
            tag: keys.recipient_tag(recipient, day_of(message.created_at)),
            nonce,
            ciphertext: Vec::new(),
        };
        mask(keys.header_key(), &nonce, &mut header);
        let sealed = XChaCha20Poly1305::new((&key).into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &[bundle.aad().as_slice(), &header].concat(),
                },
            )
            .map_err(|_| Error::Invalid("plaintext"))?;
        bundle.ciphertext = [header.as_slice(), &sealed].concat();
        *self = next;
        Ok(bundle)
    }

    /// Opens a ratchet bundle without changing this session. `skipped` looks up
    /// a stored skipped key by ratchet key and message number.
    pub fn open(
        &self,
        keys: &PairKeys,
        me: &PublicKey,
        bundle: &Bundle,
        skipped: impl Fn(&[u8; 32], u16) -> Option<[u8; 32]>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Opened> {
        if bundle.ciphertext.len() < HEADER_LEN + MESSAGE_HEADER_LEN + AEAD_TAG_LEN {
            return Err(Error::Invalid("not a ratchet message"));
        }
        let header_ct: [u8; HEADER_LEN] = truncate(&bundle.ciphertext);
        let mut header = header_ct;
        mask(keys.header_key(), &bundle.nonce, &mut header);
        if header[0] != MODE_RATCHET {
            return Err(Error::Invalid("not a ratchet message"));
        }
        let ratchet: [u8; 32] = truncate(&header[1..33]);
        let previous = u16::from_be_bytes([header[33], header[34]]);
        let n = u16::from_be_bytes([header[35], header[36]]);

        let mut next = self.clone();
        let mut new_skipped = Vec::new();
        let mut used = None;
        let key = match skipped(&ratchet, n) {
            Some(key) => {
                used = Some((ratchet, n));
                key
            }
            None => {
                if next.peer != Some(ratchet) {
                    next.skip(previous, &mut new_skipped)?;
                    next.previous = next.sent;
                    next.sent = 0;
                    next.received = 0;
                    next.peer = Some(ratchet);
                    let (root, recv) = kdf_rk(&next.root, &dh(&next.own, &ratchet)?);
                    let fresh = random_secret(rng);
                    let (root, send) = kdf_rk(&root, &dh(&fresh, &ratchet)?);
                    next.own = fresh;
                    next.root = root;
                    next.recv = Some(recv);
                    next.send = Some(send);
                }
                if n < next.received {
                    // Used already: a replay.
                    return Err(Error::Decrypt);
                }
                next.skip(n, &mut new_skipped)?;
                let (chain, key) = kdf_ck(&next.recv.ok_or(Error::Decrypt)?);
                next.recv = Some(chain);
                next.received = n + 1;
                key
            }
        };

        let plaintext = XChaCha20Poly1305::new((&key).into())
            .decrypt(
                XNonce::from_slice(&bundle.nonce),
                Payload {
                    msg: &bundle.ciphertext[HEADER_LEN..],
                    aad: &[bundle.aad().as_slice(), &header_ct].concat(),
                },
            )
            .map_err(|_| Error::Decrypt)?;
        let message = Message::from_bytes(&plaintext)?;
        check_opened(keys, me, bundle, &message)?;
        Ok(Opened {
            message,
            session: next,
            skipped: new_skipped,
            used,
        })
    }

    /// Derives and hands out the keys of the receiving chain up to `until`.
    fn skip(&mut self, until: u16, out: &mut Vec<SkippedKey>) -> Result<()> {
        let (Some(mut chain), Some(peer)) = (self.recv, self.peer) else {
            return Ok(());
        };
        if until > self.received.saturating_add(MAX_SKIP) {
            return Err(Error::Invalid("too many skipped messages"));
        }
        while self.received < until {
            let (next, key) = kdf_ck(&chain);
            out.push(SkippedKey {
                ratchet: peer,
                n: self.received,
                key,
            });
            chain = next;
            self.received += 1;
        }
        self.recv = Some(chain);
        Ok(())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(STATE_LEN);
        bytes.push(match self.role {
            Role::Initiator => 0,
            Role::Responder => 1,
        });
        bytes.extend_from_slice(&self.own);
        push_option(&mut bytes, &self.peer);
        bytes.extend_from_slice(&self.root);
        push_option(&mut bytes, &self.send);
        push_option(&mut bytes, &self.recv);
        for counter in [self.sent, self.received, self.previous] {
            bytes.extend_from_slice(&counter.to_be_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != STATE_LEN {
            return Err(Error::Invalid("session state"));
        }
        let role = match bytes[0] {
            0 => Role::Initiator,
            1 => Role::Responder,
            _ => return Err(Error::Invalid("session state")),
        };
        let option = |at: usize| -> Option<[u8; 32]> {
            (bytes[at] == 1).then(|| truncate(&bytes[at + 1..]))
        };
        let counter = |at: usize| u16::from_be_bytes([bytes[at], bytes[at + 1]]);
        Ok(Self {
            role,
            own: truncate(&bytes[1..]),
            peer: option(33),
            root: truncate(&bytes[66..]),
            send: option(98),
            recv: option(131),
            sent: counter(164),
            received: counter(166),
            previous: counter(168),
        })
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("role", &self.role)
            .field("established", &self.is_established())
            .field("sent", &self.sent)
            .field("received", &self.received)
            .finish_non_exhaustive()
    }
}

fn push_option(bytes: &mut Vec<u8>, value: &Option<[u8; 32]>) {
    bytes.push(u8::from(value.is_some()));
    bytes.extend_from_slice(&value.unwrap_or([0; 32]));
}

fn kdf_rk(root: &[u8; 32], dh: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let mut okm = [0; 64];
    Hkdf::<Sha256>::new(Some(root), dh)
        .expand(RATCHET_INFO, &mut okm)
        .expect("64 bytes is a valid HKDF-SHA256 output length");
    let keys = (truncate(&okm[..32]), truncate(&okm[32..]));
    okm.zeroize();
    keys
}

pub(crate) fn kdf_ck(chain: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    (
        hmac_sha256(chain, &[&[0x02]]),
        hmac_sha256(chain, &[&[0x01]]),
    )
}

fn dh(secret: &[u8; 32], public: &[u8; 32]) -> Result<[u8; 32]> {
    let shared = StaticSecret::from(*secret).diffie_hellman(&PublicKey::from(*public));
    if !shared.was_contributory() {
        return Err(Error::WeakKey);
    }
    Ok(*shared.as_bytes())
}

fn public(secret: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&StaticSecret::from(*secret)).to_bytes()
}

fn random_secret(rng: &mut impl CryptoRngCore) -> [u8; 32] {
    let mut secret = [0; 32];
    rng.fill_bytes(&mut secret);
    secret
}

/// XORs a header with the XChaCha20 keystream from block 0 (§5.6, §5.7).
pub(crate) fn mask(key: &[u8; 32], nonce: &[u8; 24], header: &mut [u8]) {
    XChaCha20::new(key.into(), nonce.into()).apply_keystream(header);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rand_core::OsRng;

    use super::*;
    use crate::bundle::{open as open_pair, seal as seal_pair, DEFAULT_TTL_SECS};
    use crate::frame::{frames_for, MESHTASTIC_MTU};

    const NOW: u32 = 1_790_000_000;

    struct Side {
        identity: Identity,
        keys: PairKeys,
        session: Session,
        skipped: HashMap<([u8; 32], u16), [u8; 32]>,
    }

    impl Side {
        fn send(&mut self, to: &Side, text: &str) -> Bundle {
            self.session
                .seal(
                    &self.keys,
                    &to.identity.dh_public(),
                    &Message::text(text, NOW),
                    NOW + DEFAULT_TTL_SECS,
                    8,
                    &mut OsRng,
                )
                .unwrap()
        }

        fn receive(&mut self, bundle: &Bundle) -> Result<String> {
            let skipped = &self.skipped;
            let opened = self.session.open(
                &self.keys,
                &self.identity.dh_public(),
                bundle,
                |ratchet, n| skipped.get(&(*ratchet, n)).copied(),
                &mut OsRng,
            )?;
            if let Some(used) = opened.used {
                self.skipped.remove(&used);
            }
            for key in &opened.skipped {
                self.skipped.insert((key.ratchet, key.n), key.key);
            }
            self.session = opened.session;
            Ok(String::from_utf8(opened.message.body).unwrap())
        }
    }

    fn side(me: Identity, peer: &PublicKey) -> Side {
        let keys = PairKeys::derive(&me, peer).unwrap();
        let session = Session::new(&me, peer, &keys);
        Side {
            identity: me,
            keys,
            session,
            skipped: HashMap::new(),
        }
    }

    /// The initiator and the responder of a fresh pair.
    fn pair() -> (Side, Side) {
        let a = Identity::generate("a").unwrap();
        let b = Identity::generate("b").unwrap();
        let (a_key, b_key) = (a.dh_public(), b.dh_public());
        let (first, second) = (side(a, &b_key), side(b, &a_key));
        if first.session.role() == Role::Initiator {
            (first, second)
        } else {
            (second, first)
        }
    }

    #[test]
    fn only_the_initiator_starts() {
        let (initiator, mut responder) = pair();
        assert_eq!(initiator.session.role(), Role::Initiator);
        assert!(!responder.session.can_send());
        let result = responder.session.seal(
            &responder.keys,
            &initiator.identity.dh_public(),
            &Message::text("hi", NOW),
            NOW + 60,
            8,
            &mut OsRng,
        );
        assert_eq!(result.unwrap_err(), Error::NoSession);
    }

    #[test]
    fn messages_open_out_of_order_and_both_ways() {
        let (mut a, mut b) = pair();
        let first = a.send(&b, "bir");
        let second = a.send(&b, "iki");
        let third = a.send(&b, "üç");
        assert_eq!(b.receive(&third).unwrap(), "üç");
        assert_eq!(b.receive(&first).unwrap(), "bir");
        assert_eq!(b.receive(&second).unwrap(), "iki");
        assert!(b.skipped.is_empty());
        assert!(b.session.can_send());

        for round in 0..5 {
            let reply = b.send(&a, &format!("cevap {round}"));
            assert_eq!(a.receive(&reply).unwrap(), format!("cevap {round}"));
            let next = a.send(&b, &format!("soru {round}"));
            assert_eq!(b.receive(&next).unwrap(), format!("soru {round}"));
        }
        assert!(a.session.is_established() && b.session.is_established());
    }

    #[test]
    fn a_late_message_from_an_old_chain_still_opens() {
        let (mut a, mut b) = pair();
        let early = a.send(&b, "geç kalan");
        let later = a.send(&b, "önce gelen");
        assert_eq!(b.receive(&later).unwrap(), "önce gelen");
        let reply = b.send(&a, "tamam");
        a.receive(&reply).unwrap();
        let newer = a.send(&b, "yeni zincir");
        assert_eq!(b.receive(&newer).unwrap(), "yeni zincir");
        assert_eq!(b.receive(&early).unwrap(), "geç kalan");
    }

    #[test]
    fn a_message_opens_once() {
        let (mut a, mut b) = pair();
        let bundle = a.send(&b, "bir kez");
        b.receive(&bundle).unwrap();
        assert_eq!(b.receive(&bundle).unwrap_err(), Error::Decrypt);
    }

    #[test]
    fn any_changed_byte_is_refused() {
        let (mut a, b) = pair();
        let bundle = a.send(&b, "hi");
        for index in [0, 1, 20, 36, 37, bundle.ciphertext.len() - 1] {
            let mut changed = bundle.clone();
            changed.ciphertext[index] ^= 0x01;
            let result = b.session.open(
                &b.keys,
                &b.identity.dh_public(),
                &changed,
                |_, _| None,
                &mut OsRng,
            );
            assert!(result.is_err(), "byte {index}");
        }
    }

    #[test]
    fn ratchet_and_static_bundles_do_not_mix() {
        let (mut a, b) = pair();
        let ratchet = a.send(&b, "ratchet");
        assert!(open_pair(&b.keys, &b.identity.dh_public(), &ratchet).is_err());
        let fixed = seal_pair(
            &a.keys,
            &b.identity.dh_public(),
            &Message::text("K_enc", NOW),
            NOW + 60,
            8,
        )
        .unwrap();
        let result = b.session.open(
            &b.keys,
            &b.identity.dh_public(),
            &fixed,
            |_, _| None,
            &mut OsRng,
        );
        assert!(result.is_err());
    }

    #[test]
    fn state_survives_a_round_trip() {
        let (mut a, mut b) = pair();
        b.receive(&a.send(&b, "hi")).unwrap();
        let restored = Session::from_bytes(&b.session.to_bytes()).unwrap();
        assert!(restored == b.session);
    }

    #[test]
    fn a_short_ratchet_message_fits_one_meshtastic_packet() {
        let (mut a, b) = pair();
        let bundle = a.send(&b, &"a".repeat(MESHTASTIC_MTU - 1 - OVERHEAD));
        assert_eq!(frames_for(&bundle, MESHTASTIC_MTU).unwrap().len(), 1);
        assert_eq!(MESHTASTIC_MTU - 1 - OVERHEAD, 128);
    }
}

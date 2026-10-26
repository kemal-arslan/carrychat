//! Groups (§5.7): every member seals with its own sender chain, which it hands
//! to the other members in pair bundles, and signs what it seals, so one
//! bundle reaches every member and no member can write as another.
//!
//! Nothing here stores anything: [`ReceiverChain::open`] works on a copy and
//! returns the new state with the keys to keep, for the caller to commit only
//! when it accepts the message.

use core::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand_core::CryptoRngCore;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::bundle::{
    check_lifetime, Bundle, BundleId, Kind, Message, AEAD_TAG_LEN, HEADER_LEN as BUNDLE_HEADER_LEN,
    MAX_BUNDLE_LEN, MESSAGE_HEADER_LEN,
};
use crate::identity::{ContactCard, MAX_NAME_LEN};
use crate::pair::RecipientTag;
use crate::ratchet::{kdf_ck, mask};
use crate::{day_of, hmac_sha256, sha256, truncate, Error, Result};

pub const GROUP_ID_LEN: usize = 16;
/// Members of a group at most, the admin included.
pub const MAX_MEMBERS: usize = 16;
pub const MAX_GROUP_NAME_LEN: usize = MAX_NAME_LEN;
/// The encrypted iteration in front of the sealed message.
pub const HEADER_LEN: usize = 4;
pub const SIGNATURE_LEN: usize = 64;
/// Bytes a group bundle adds around a message body.
pub const OVERHEAD: usize =
    BUNDLE_HEADER_LEN + HEADER_LEN + MESSAGE_HEADER_LEN + AEAD_TAG_LEN + SIGNATURE_LEN;
/// Message keys a receiver derives ahead for messages that have not arrived.
pub const MAX_SKIP: u32 = 1000;
/// How far ahead of what it holds a receiver derives a chain for one message.
/// It keeps the keys of only the last [`MAX_SKIP`] it passes.
pub const MAX_JUMP: u32 = 100_000;
pub const DISTRIBUTION_LEN: usize = GROUP_ID_LEN + 32 + 4 + 32 + 32;

const SALT: &[u8] = b"carrychat/v1/group";
const BUNDLE_ID_LABEL: &[u8] = b"carrychat/v1/group";
const SIGNATURE_LABEL: &[u8] = b"carrychat/v1/group-signature";
const CHAIN_STATE_LEN: usize = 32 + 32 + 4 + 32;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct GroupId(pub [u8; GROUP_ID_LEN]);

impl GroupId {
    pub fn generate(rng: &mut impl CryptoRngCore) -> Self {
        let mut id = [0; GROUP_ID_LEN];
        rng.fill_bytes(&mut id);
        Self(id)
    }
}

/// Who is in a group and what it is called, as its admin last sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupState {
    pub id: GroupId,
    pub version: u32,
    pub name: String,
    /// The members, the admin included.
    pub members: Vec<ContactCard>,
}

impl GroupState {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.check()?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.id.0);
        bytes.extend_from_slice(&self.version.to_be_bytes());
        bytes.push(self.name.len() as u8);
        bytes.extend_from_slice(self.name.as_bytes());
        bytes.push(self.members.len() as u8);
        for card in &self.members {
            let card = card.to_bytes();
            bytes.push(card.len() as u8);
            bytes.extend_from_slice(&card);
        }
        Ok(bytes)
    }

    /// Parses a state and verifies every card in it.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader(bytes);
        let id = GroupId(truncate(reader.take(GROUP_ID_LEN)?));
        let version = u32::from_be_bytes(truncate(reader.take(4)?));
        let name_len = reader.take(1)?[0] as usize;
        let name = String::from_utf8(reader.take(name_len)?.to_vec())
            .map_err(|_| Error::Invalid("group name"))?;
        let count = reader.take(1)?[0] as usize;
        let mut members = Vec::with_capacity(count);
        for _ in 0..count {
            let card_len = reader.take(1)?[0] as usize;
            members.push(ContactCard::from_bytes(reader.take(card_len)?)?);
        }
        if !reader.0.is_empty() {
            return Err(Error::TooLong);
        }
        let state = Self {
            id,
            version,
            name,
            members,
        };
        state.check()?;
        Ok(state)
    }

    fn check(&self) -> Result<()> {
        if self.name.len() > MAX_GROUP_NAME_LEN {
            return Err(Error::Invalid("group name length"));
        }
        if self.members.is_empty() || self.members.len() > MAX_MEMBERS {
            return Err(Error::Invalid("number of members"));
        }
        for (index, card) in self.members.iter().enumerate() {
            if self.members[..index]
                .iter()
                .any(|other| other.dh_key() == card.dh_key())
            {
                return Err(Error::Invalid("member listed twice"));
            }
        }
        Ok(())
    }
}

/// A member's sender chain as it hands it to the other members (§5.7).
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct Distribution {
    #[zeroize(skip)]
    pub group: GroupId,
    pub sender_key: [u8; 32],
    pub iteration: u32,
    pub chain: [u8; 32],
    pub verifying_key: [u8; 32],
}

impl Distribution {
    pub fn to_bytes(&self) -> [u8; DISTRIBUTION_LEN] {
        let mut bytes = [0; DISTRIBUTION_LEN];
        bytes[..16].copy_from_slice(&self.group.0);
        bytes[16..48].copy_from_slice(&self.sender_key);
        bytes[48..52].copy_from_slice(&self.iteration.to_be_bytes());
        bytes[52..84].copy_from_slice(&self.chain);
        bytes[84..].copy_from_slice(&self.verifying_key);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        match bytes.len().cmp(&DISTRIBUTION_LEN) {
            core::cmp::Ordering::Less => return Err(Error::Truncated),
            core::cmp::Ordering::Greater => return Err(Error::TooLong),
            core::cmp::Ordering::Equal => {}
        }
        let distribution = Self {
            group: GroupId(truncate(bytes)),
            sender_key: truncate(&bytes[16..]),
            iteration: u32::from_be_bytes(truncate(&bytes[48..])),
            chain: truncate(&bytes[52..]),
            verifying_key: truncate(&bytes[84..]),
        };
        VerifyingKey::from_bytes(&distribution.verifying_key)
            .map_err(|_| Error::Invalid("signing key"))?;
        Ok(distribution)
    }
}

impl fmt::Debug for Distribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Distribution")
            .field("group", &self.group)
            .field("iteration", &self.iteration)
            .finish_non_exhaustive()
    }
}

impl Message {
    /// A group state, from the group's admin (§5.7).
    pub fn group(state: &GroupState, created_at: u32) -> Result<Self> {
        Ok(Self {
            kind: Kind::Group,
            created_at,
            body: state.to_bytes()?,
        })
    }

    /// The sender's chain for a group (§5.7).
    pub fn sender_key(distribution: &Distribution, created_at: u32) -> Self {
        Self {
            kind: Kind::SenderKey,
            created_at,
            body: distribution.to_bytes().to_vec(),
        }
    }

    /// The sender leaves the group (§5.7).
    pub fn leave(group: GroupId, created_at: u32) -> Self {
        Self {
            kind: Kind::Leave,
            created_at,
            body: group.0.to_vec(),
        }
    }
}

/// Our own chain for a group: it seals and signs our group messages.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct SenderChain {
    sender_key: [u8; 32],
    chain: [u8; 32],
    iteration: u32,
    signing_key: [u8; 32],
}

impl SenderChain {
    pub fn generate(rng: &mut impl CryptoRngCore) -> Self {
        let mut secrets = [[0; 32]; 3];
        for secret in &mut secrets {
            rng.fill_bytes(secret);
        }
        let [sender_key, chain, signing_key] = secrets;
        Self {
            sender_key,
            chain,
            iteration: 0,
            signing_key,
        }
    }

    /// The chain as it stands, for another member of `group`.
    pub fn distribution(&self, group: GroupId) -> Distribution {
        Distribution {
            group,
            sender_key: self.sender_key,
            iteration: self.iteration,
            chain: self.chain,
            verifying_key: SigningKey::from_bytes(&self.signing_key)
                .verifying_key()
                .to_bytes(),
        }
    }

    pub fn tag(&self, day: u32) -> RecipientTag {
        ChainKeys::derive(&self.sender_key).tag(day)
    }

    /// Identifies the chain without revealing it, as [`ReceiverChain::id`].
    pub fn id(&self) -> [u8; 32] {
        chain_id(&self.sender_key)
    }

    pub fn iteration(&self) -> u32 {
        self.iteration
    }

    pub fn seal(
        &mut self,
        message: &Message,
        expires_at: u32,
        hop_limit: u8,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Bundle> {
        let mut nonce = [0; 24];
        rng.fill_bytes(&mut nonce);
        self.seal_with_nonce(message, expires_at, hop_limit, nonce)
    }

    /// Seals with a caller-chosen nonce, for test vectors. Use [`SenderChain::seal`].
    pub fn seal_with_nonce(
        &mut self,
        message: &Message,
        expires_at: u32,
        hop_limit: u8,
        nonce: [u8; 24],
    ) -> Result<Bundle> {
        check_lifetime(message.created_at, expires_at)?;
        let plaintext = message.to_bytes();
        if OVERHEAD - MESSAGE_HEADER_LEN + plaintext.len() > MAX_BUNDLE_LEN {
            return Err(Error::TooLong);
        }
        let iteration = self.iteration;
        let next = iteration
            .checked_add(1)
            .ok_or(Error::Invalid("the chain is used up"))?;
        let keys = ChainKeys::derive(&self.sender_key);
        let (chain, key) = kdf_ck(&self.chain);

        let mut bundle = Bundle {
            hop_limit,
            expires_at,
            id: bundle_id(&nonce),
            tag: keys.tag(day_of(message.created_at)),
            nonce,
            ciphertext: Vec::new(),
        };
        let mut header = iteration.to_be_bytes();
        mask(&keys.header, &nonce, &mut header);
        let aad = [bundle.aad().as_slice(), &header].concat();
        let sealed = XChaCha20Poly1305::new((&key).into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Invalid("plaintext"))?;
        let signature = SigningKey::from_bytes(&self.signing_key)
            .sign(&[SIGNATURE_LABEL, &aad, &sealed].concat());
        bundle.ciphertext = [header.as_slice(), &sealed, &signature.to_bytes()].concat();
        self.chain = chain;
        self.iteration = next;
        Ok(bundle)
    }

    pub fn to_bytes(&self) -> [u8; CHAIN_STATE_LEN] {
        chain_state(
            &self.sender_key,
            &self.chain,
            self.iteration,
            &self.signing_key,
        )
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let (sender_key, chain, iteration, signing_key) = parse_chain_state(bytes)?;
        Ok(Self {
            sender_key,
            chain,
            iteration,
            signing_key,
        })
    }
}

impl fmt::Debug for SenderChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SenderChain")
            .field("iteration", &self.iteration)
            .finish_non_exhaustive()
    }
}

/// A key derived ahead for a message that has not arrived.
pub type SkippedGroupKey = (u32, [u8; 32]);

/// What opening a group bundle gives: the message, and the chain and keys to
/// commit if the caller accepts it.
pub struct GroupOpened {
    pub message: Message,
    pub chain: ReceiverChain,
    pub skipped: Vec<SkippedGroupKey>,
    /// The stored skipped key that was used up, by iteration.
    pub used: Option<u32>,
}

/// Another member's chain, as a receiver keeps it.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct ReceiverChain {
    sender_key: [u8; 32],
    chain: [u8; 32],
    /// The next iteration the chain key can derive.
    iteration: u32,
    verifying_key: [u8; 32],
}

impl ReceiverChain {
    pub fn new(distribution: &Distribution) -> Self {
        Self {
            sender_key: distribution.sender_key,
            chain: distribution.chain,
            iteration: distribution.iteration,
            verifying_key: distribution.verifying_key,
        }
    }

    /// Identifies the chain without revealing it: hashes its sender key.
    pub fn id(&self) -> [u8; 32] {
        chain_id(&self.sender_key)
    }

    pub fn tag(&self, day: u32) -> RecipientTag {
        ChainKeys::derive(&self.sender_key).tag(day)
    }

    /// Opens a group bundle without changing this chain. `skipped` looks up a
    /// stored skipped key by iteration.
    pub fn open(
        &self,
        bundle: &Bundle,
        skipped: impl Fn(u32) -> Option<[u8; 32]>,
    ) -> Result<GroupOpened> {
        let length = bundle.ciphertext.len();
        if length < HEADER_LEN + MESSAGE_HEADER_LEN + AEAD_TAG_LEN + SIGNATURE_LEN {
            return Err(Error::Invalid("not a group message"));
        }
        let (body, signature) = bundle.ciphertext.split_at(length - SIGNATURE_LEN);
        let aad = [bundle.aad().as_slice(), &body[..HEADER_LEN]].concat();
        VerifyingKey::from_bytes(&self.verifying_key)
            .map_err(|_| Error::Invalid("signing key"))?
            .verify_strict(
                &[SIGNATURE_LABEL, &aad, &body[HEADER_LEN..]].concat(),
                &Signature::from_bytes(&truncate(signature)),
            )
            .map_err(|_| Error::BadSignature)?;

        let keys = ChainKeys::derive(&self.sender_key);
        let mut header: [u8; HEADER_LEN] = truncate(body);
        mask(&keys.header, &bundle.nonce, &mut header);
        let iteration = u32::from_be_bytes(header);

        let mut next = self.clone();
        let mut new_skipped = Vec::new();
        let mut used = None;
        let key = match skipped(iteration) {
            Some(key) => {
                used = Some(iteration);
                key
            }
            None => {
                if iteration < next.iteration {
                    // Used already, or from before we got the chain.
                    return Err(Error::Decrypt);
                }
                if iteration - next.iteration > MAX_JUMP {
                    return Err(Error::Invalid("too many skipped messages"));
                }
                let keep_from = iteration.saturating_sub(MAX_SKIP);
                while next.iteration < iteration {
                    let (chain, key) = kdf_ck(&next.chain);
                    if next.iteration >= keep_from {
                        new_skipped.push((next.iteration, key));
                    }
                    next.chain = chain;
                    next.iteration += 1;
                }
                let (chain, key) = kdf_ck(&next.chain);
                next.chain = chain;
                next.iteration = iteration
                    .checked_add(1)
                    .ok_or(Error::Invalid("the chain is used up"))?;
                key
            }
        };

        let plaintext = XChaCha20Poly1305::new((&key).into())
            .decrypt(
                XNonce::from_slice(&bundle.nonce),
                Payload {
                    msg: &body[HEADER_LEN..],
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Decrypt)?;
        let message = Message::from_bytes(&plaintext)?;
        if keys.tag(day_of(message.created_at)) != bundle.tag {
            return Err(Error::Invalid("group tag"));
        }
        check_lifetime(message.created_at, bundle.expires_at)?;
        if bundle_id(&bundle.nonce) != bundle.id {
            return Err(Error::Invalid("bundle id"));
        }
        Ok(GroupOpened {
            message,
            chain: next,
            skipped: new_skipped,
            used,
        })
    }

    pub fn to_bytes(&self) -> [u8; CHAIN_STATE_LEN] {
        chain_state(
            &self.sender_key,
            &self.chain,
            self.iteration,
            &self.verifying_key,
        )
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let (sender_key, chain, iteration, verifying_key) = parse_chain_state(bytes)?;
        Ok(Self {
            sender_key,
            chain,
            iteration,
            verifying_key,
        })
    }
}

impl fmt::Debug for ReceiverChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReceiverChain")
            .field("iteration", &self.iteration)
            .finish_non_exhaustive()
    }
}

/// The tag and header keys of a chain, from its sender key (§5.7).
struct ChainKeys {
    tag: [u8; 32],
    header: [u8; 32],
}

impl ChainKeys {
    fn derive(sender_key: &[u8; 32]) -> Self {
        let hkdf = Hkdf::<Sha256>::new(Some(SALT), sender_key);
        let mut keys = Self {
            tag: [0; 32],
            header: [0; 32],
        };
        hkdf.expand(b"tag", &mut keys.tag)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        hkdf.expand(b"hdr", &mut keys.header)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        keys
    }

    fn tag(&self, day: u32) -> RecipientTag {
        truncate(&hmac_sha256(&self.tag, &[&day.to_be_bytes()]))
    }
}

impl Drop for ChainKeys {
    fn drop(&mut self) {
        self.tag.zeroize();
        self.header.zeroize();
    }
}

fn chain_id(sender_key: &[u8; 32]) -> [u8; 32] {
    sha256(&[b"carrychat/v1/group-chain", sender_key])
}

fn bundle_id(nonce: &[u8; 24]) -> BundleId {
    BundleId(truncate(&sha256(&[BUNDLE_ID_LABEL, nonce])))
}

fn chain_state(
    sender_key: &[u8; 32],
    chain: &[u8; 32],
    iteration: u32,
    signing: &[u8; 32],
) -> [u8; CHAIN_STATE_LEN] {
    let mut bytes = [0; CHAIN_STATE_LEN];
    bytes[..32].copy_from_slice(sender_key);
    bytes[32..64].copy_from_slice(chain);
    bytes[64..68].copy_from_slice(&iteration.to_be_bytes());
    bytes[68..].copy_from_slice(signing);
    bytes
}

/// Sender key, chain key, iteration, and the signing or verifying key.
type ChainParts = ([u8; 32], [u8; 32], u32, [u8; 32]);

fn parse_chain_state(bytes: &[u8]) -> Result<ChainParts> {
    if bytes.len() != CHAIN_STATE_LEN {
        return Err(Error::Invalid("chain state"));
    }
    Ok((
        truncate(bytes),
        truncate(&bytes[32..]),
        u32::from_be_bytes(truncate(&bytes[64..])),
        truncate(&bytes[68..]),
    ))
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.0.len() < len {
            return Err(Error::Truncated);
        }
        let (taken, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(taken)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rand_core::OsRng;

    use super::*;
    use crate::bundle::DEFAULT_TTL_SECS;
    use crate::frame::{frames_for, MESHTASTIC_MTU};
    use crate::identity::Identity;

    const NOW: u32 = 1_790_000_000;
    const GROUP: GroupId = GroupId([7; GROUP_ID_LEN]);

    /// A member's view of one sender's chain, with its stored skipped keys.
    struct Member {
        chain: ReceiverChain,
        skipped: HashMap<u32, [u8; 32]>,
    }

    impl Member {
        fn of(sender: &SenderChain) -> Self {
            Self {
                chain: ReceiverChain::new(&sender.distribution(GROUP)),
                skipped: HashMap::new(),
            }
        }

        fn receive(&mut self, bundle: &Bundle) -> Result<String> {
            let skipped = &self.skipped;
            let opened = self.chain.open(bundle, |n| skipped.get(&n).copied())?;
            if let Some(used) = opened.used {
                self.skipped.remove(&used);
            }
            self.skipped.extend(opened.skipped.iter().copied());
            self.chain = opened.chain;
            Ok(String::from_utf8(opened.message.body).unwrap())
        }
    }

    fn send(sender: &mut SenderChain, text: &str) -> Bundle {
        sender
            .seal(
                &Message::text(text, NOW),
                NOW + DEFAULT_TTL_SECS,
                8,
                &mut OsRng,
            )
            .unwrap()
    }

    #[test]
    fn every_member_reads_the_one_bundle() {
        let mut alice = SenderChain::generate(&mut OsRng);
        let (mut bob, mut carol) = (Member::of(&alice), Member::of(&alice));
        let bundle =
            Bundle::from_bytes(&send(&mut alice, "Toplanma yeri okul").to_bytes()).unwrap();
        assert_eq!(bob.receive(&bundle).unwrap(), "Toplanma yeri okul");
        assert_eq!(carol.receive(&bundle).unwrap(), "Toplanma yeri okul");
        assert_eq!(bundle.tag, alice.tag(day_of(NOW)));
    }

    #[test]
    fn late_messages_open_once_and_in_any_order() {
        let mut alice = SenderChain::generate(&mut OsRng);
        let mut bob = Member::of(&alice);
        let bundles: Vec<Bundle> = (0..4).map(|n| send(&mut alice, &format!("{n}"))).collect();
        assert_eq!(bob.receive(&bundles[3]).unwrap(), "3");
        assert_eq!(bob.skipped.len(), 3);
        assert_eq!(bob.receive(&bundles[1]).unwrap(), "1");
        assert_eq!(bob.receive(&bundles[0]).unwrap(), "0");
        assert_eq!(bob.receive(&bundles[2]).unwrap(), "2");
        assert!(bob.skipped.is_empty());
        for bundle in &bundles {
            assert_eq!(bob.receive(bundle).unwrap_err(), Error::Decrypt);
        }
    }

    #[test]
    fn a_new_member_reads_nothing_sent_before_it_joined() {
        let mut alice = SenderChain::generate(&mut OsRng);
        let before = send(&mut alice, "eski");
        let mut dave = Member::of(&alice);
        assert_eq!(dave.receive(&before).unwrap_err(), Error::Decrypt);
        let after = send(&mut alice, "yeni");
        assert_eq!(dave.receive(&after).unwrap(), "yeni");
    }

    #[test]
    fn a_member_cannot_write_as_another() {
        let mut alice = SenderChain::generate(&mut OsRng);
        let mut bob = Member::of(&alice);
        // Bob holds Alice's chain and tags, but not her signing key.
        let mut forged = alice.clone();
        forged.signing_key = SenderChain::generate(&mut OsRng).signing_key;
        let bundle = send(&mut forged, "ben Alice'im");
        assert_eq!(bob.receive(&bundle).unwrap_err(), Error::BadSignature);
        assert_eq!(bob.receive(&send(&mut alice, "asıl")).unwrap(), "asıl");
    }

    #[test]
    fn any_changed_byte_is_refused() {
        let mut alice = SenderChain::generate(&mut OsRng);
        let bob = Member::of(&alice);
        let original = send(&mut alice, "hi").to_bytes();
        for index in (0..original.len()).filter(|&i| i != 1) {
            let mut bytes = original.clone();
            bytes[index] ^= 0x01;
            let Ok(bundle) = Bundle::from_bytes(&bytes) else {
                continue;
            };
            assert!(bob.chain.open(&bundle, |_| None).is_err(), "byte {index}");
        }
    }

    #[test]
    fn a_long_gap_opens_and_keeps_only_recent_keys() {
        let mut alice = SenderChain::generate(&mut OsRng);
        let mut bob = Member::of(&alice);
        let lost: Vec<Bundle> = (0..1005)
            .map(|n| send(&mut alice, &format!("{n}")))
            .collect();
        assert_eq!(bob.receive(&send(&mut alice, "sonra")).unwrap(), "sonra");
        assert_eq!(bob.skipped.len(), MAX_SKIP as usize);
        assert_eq!(bob.receive(&lost[1004]).unwrap(), "1004");
        assert!(bob.receive(&lost[0]).is_err());
    }

    #[test]
    fn a_jump_too_far_ahead_is_refused() {
        let mut alice = SenderChain::generate(&mut OsRng);
        let mut bob = Member::of(&alice);
        alice.iteration += MAX_JUMP + 1;
        let far = send(&mut alice, "uzak");
        assert!(bob.receive(&far).is_err());
    }

    #[test]
    fn states_and_chains_survive_a_round_trip() {
        let cards: Vec<ContactCard> = ["Ayşe", "Deniz", "Mehmet"]
            .iter()
            .map(|name| Identity::generate(name).unwrap().card())
            .collect();
        let state = GroupState {
            id: GROUP,
            version: 3,
            name: "Mahalle".into(),
            members: cards.clone(),
        };
        let bytes = state.to_bytes().unwrap();
        assert_eq!(GroupState::from_bytes(&bytes).unwrap(), state);
        assert!(GroupState::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        assert!(GroupState::from_bytes(&[bytes.as_slice(), &[0]].concat()).is_err());

        let twice = GroupState {
            members: vec![cards[0].clone(), cards[0].clone()],
            ..state.clone()
        };
        assert!(twice.to_bytes().is_err());
        let crowded = GroupState {
            members: (0..=MAX_MEMBERS)
                .map(|_| Identity::generate("x").unwrap().card())
                .collect(),
            ..state.clone()
        };
        assert!(crowded.to_bytes().is_err());

        let mut alice = SenderChain::generate(&mut OsRng);
        send(&mut alice, "hi");
        assert_eq!(SenderChain::from_bytes(&alice.to_bytes()).unwrap(), alice);
        let distribution = alice.distribution(GROUP);
        assert_eq!(
            Distribution::from_bytes(&distribution.to_bytes()).unwrap(),
            distribution
        );
        let bob = ReceiverChain::new(&distribution);
        assert_eq!(ReceiverChain::from_bytes(&bob.to_bytes()).unwrap(), bob);
    }

    #[test]
    fn a_short_group_message_fits_one_meshtastic_packet() {
        let mut alice = SenderChain::generate(&mut OsRng);
        let text = "a".repeat(MESHTASTIC_MTU - 1 - OVERHEAD);
        assert_eq!(text.len(), 97);
        let bundle = send(&mut alice, &text);
        assert_eq!(frames_for(&bundle, MESHTASTIC_MTU).unwrap().len(), 1);
        let longer = send(&mut alice, &format!("{text}a"));
        assert_eq!(frames_for(&longer, MESHTASTIC_MTU).unwrap().len(), 2);
    }
}

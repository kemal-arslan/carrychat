//! Reticulum's wire format, as far as the link uses it (§9.9): identities,
//! destination hashes, packets, announces, path requests, encryption to a
//! single destination, and the HDLC framing of its TCP interfaces. Follows
//! the reference implementation, RNS 1.5.5.

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes256;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

/// Destination and identity hashes are truncated SHA-256.
pub const HASH_LEN: usize = 16;
const NAME_HASH_LEN: usize = 10;
const RANDOM_HASH_LEN: usize = 10;
const PUBLIC_KEY_LEN: usize = 64;
const SIGNATURE_LEN: usize = 64;
/// The most plaintext one packet encrypted to a single destination carries,
/// with room for a transport node's header (`Packet.ENCRYPTED_MDU`).
pub const MTU: usize = 383;
/// CarryChat's destination: app `carrychat`, aspect `frames`.
pub const NAME: &str = "carrychat.frames";
/// The plain destination path requests go to.
const PATH_REQUEST: &str = "rnstransport.path.request";

const DATA: u8 = 0x00;
const ANNOUNCE: u8 = 0x01;
const SINGLE: u8 = 0x00;
const PLAIN: u8 = 0x02;
const HEADER_2: u8 = 0x40;
const TRANSPORT: u8 = 0x10;
const CONTEXT_FLAG: u8 = 0x20;
const IFAC: u8 = 0x80;
const CONTEXT_NONE: u8 = 0x00;
const CONTEXT_PATH_RESPONSE: u8 = 0x0B;

/// Frames on Reticulum's TCP interfaces: HDLC-like byte stuffing.
const FLAG: u8 = 0x7E;
const ESCAPE: u8 = 0x7D;
const ESCAPE_MASK: u8 = 0x20;
/// Shorter frames are ignored, as Reticulum does.
const MIN_FRAME_LEN: usize = 20;
/// Longer ones are not Reticulum's.
const MAX_FRAME_LEN: usize = 1 << 18;

fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn truncated(hash: [u8; 32]) -> [u8; HASH_LEN] {
    hash[..HASH_LEN].try_into().expect("16 of 32 bytes")
}

/// The first ten bytes of SHA-256 of a destination's name.
pub fn name_hash(name: &str) -> [u8; NAME_HASH_LEN] {
    sha256(&[name.as_bytes()])[..NAME_HASH_LEN]
        .try_into()
        .expect("10 of 32 bytes")
}

/// A single destination's hash: its name hash and its identity's hash.
pub fn destination_hash(name: &str, identity_hash: &[u8; HASH_LEN]) -> [u8; HASH_LEN] {
    truncated(sha256(&[&name_hash(name), identity_hash]))
}

/// A plain destination's hash, which has no identity.
fn plain_destination_hash(name: &str) -> [u8; HASH_LEN] {
    truncated(sha256(&[&name_hash(name)]))
}

fn identity_hash(public_key: &[u8; PUBLIC_KEY_LEN]) -> [u8; HASH_LEN] {
    truncated(sha256(&[public_key]))
}

/// A Reticulum identity: an X25519 key for encryption and an Ed25519 key
/// for signing. Its public key is the X25519 key, then the Ed25519 key.
pub struct Identity {
    encryption: StaticSecret,
    signing: SigningKey,
}

impl Identity {
    /// From the X25519 secret and the Ed25519 seed, as Reticulum stores
    /// them.
    pub fn from_secret(secret: &[u8; 64]) -> Self {
        let encryption: [u8; 32] = secret[..32].try_into().expect("32 bytes");
        let signing: [u8; 32] = secret[32..].try_into().expect("32 bytes");
        Self {
            encryption: StaticSecret::from(encryption),
            signing: SigningKey::from_bytes(&signing),
        }
    }

    pub fn public_key(&self) -> [u8; PUBLIC_KEY_LEN] {
        let mut key = [0; PUBLIC_KEY_LEN];
        key[..32].copy_from_slice(PublicKey::from(&self.encryption).as_bytes());
        key[32..].copy_from_slice(self.signing.verifying_key().as_bytes());
        key
    }

    pub fn hash(&self) -> [u8; HASH_LEN] {
        identity_hash(&self.public_key())
    }

    /// This identity's CarryChat destination.
    pub fn destination(&self) -> [u8; HASH_LEN] {
        destination_hash(NAME, &self.hash())
    }

    /// The plaintext of a packet encrypted to this identity, if it opens.
    pub fn decrypt(&self, data: &[u8]) -> Option<Vec<u8>> {
        if data.len() <= 32 + 32 {
            return None;
        }
        let ephemeral: [u8; 32] = data[..32].try_into().ok()?;
        let shared = self.encryption.diffie_hellman(&PublicKey::from(ephemeral));
        let (signing, encryption) = derive(shared.as_bytes(), &self.hash());
        let token = &data[32..];
        let (signed, tag) = token.split_at(token.len() - 32);
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&signing).expect("any key length");
        mac.update(signed);
        mac.verify_slice(tag).ok()?;
        let (iv, ciphertext) = signed.split_at(16);
        if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
            return None;
        }
        let cipher = Aes256::new(&encryption.into());
        let mut previous: [u8; 16] = iv.try_into().ok()?;
        let mut plaintext = Vec::with_capacity(ciphertext.len());
        for block in ciphertext.chunks(16) {
            let mut decrypted = aes::Block::clone_from_slice(block);
            cipher.decrypt_block(&mut decrypted);
            plaintext.extend(
                decrypted
                    .iter()
                    .zip(previous)
                    .map(|(byte, chain)| byte ^ chain),
            );
            previous = block.try_into().ok()?;
        }
        let pad = usize::from(*plaintext.last()?);
        if pad == 0 || pad > 16 {
            return None;
        }
        plaintext.truncate(plaintext.len() - pad);
        Some(plaintext)
    }
}

/// The HMAC key and the AES-256 key for a shared secret, with the
/// recipient's identity hash as the salt.
fn derive(shared: &[u8; 32], recipient: &[u8; HASH_LEN]) -> ([u8; 32], [u8; 32]) {
    let mut derived = [0; 64];
    Hkdf::<Sha256>::new(Some(recipient), shared)
        .expand(&[], &mut derived)
        .expect("64 bytes is a valid length");
    (
        derived[..32].try_into().expect("32 bytes"),
        derived[32..].try_into().expect("32 bytes"),
    )
}

/// `plaintext` encrypted to the identity with `public_key`: a fresh
/// ephemeral X25519 key, then the token: IV, AES-256-CBC ciphertext and
/// HMAC-SHA256.
pub fn encrypt(public_key: &[u8; PUBLIC_KEY_LEN], plaintext: &[u8]) -> Vec<u8> {
    let ephemeral = StaticSecret::random_from_rng(OsRng);
    let peer: [u8; 32] = public_key[..32].try_into().expect("32 bytes");
    let shared = ephemeral.diffie_hellman(&PublicKey::from(peer));
    let (signing, encryption) = derive(shared.as_bytes(), &identity_hash(public_key));
    let mut iv = [0; 16];
    OsRng.fill_bytes(&mut iv);
    let pad = 16 - plaintext.len() % 16;
    let mut padded = plaintext.to_vec();
    padded.resize(plaintext.len() + pad, pad as u8);
    let cipher = Aes256::new(&encryption.into());
    let mut out = PublicKey::from(&ephemeral).as_bytes().to_vec();
    out.extend_from_slice(&iv);
    let mut previous = iv;
    for block in padded.chunks(16) {
        let mut chained = aes::Block::default();
        for (byte, (plain, chain)) in chained.iter_mut().zip(block.iter().zip(previous)) {
            *byte = plain ^ chain;
        }
        cipher.encrypt_block(&mut chained);
        previous = chained.into();
        out.extend_from_slice(&previous);
    }
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&signing).expect("any key length");
    mac.update(&out[32..]);
    out.extend_from_slice(&mac.finalize().into_bytes());
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketType {
    Data,
    Announce,
    LinkRequest,
    Proof,
}

/// A packet as it came off the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    raw: Vec<u8>,
    pub packet_type: PacketType,
    /// Whether the destination is a plain one, as path requests' is.
    pub plain: bool,
    pub context_flag: bool,
    pub hops: u8,
    /// The next transport node, in a packet with the second header.
    pub transport_id: Option<[u8; HASH_LEN]>,
    pub destination: [u8; HASH_LEN],
    pub context: u8,
    pub data: Vec<u8>,
}

impl Packet {
    pub fn parse(raw: &[u8]) -> Option<Self> {
        let flags = *raw.first()?;
        if flags & IFAC != 0 {
            return None;
        }
        let header_2 = flags & HEADER_2 != 0;
        let (transport_id, at) = if header_2 {
            (
                Some(raw.get(2..2 + HASH_LEN)?.try_into().ok()?),
                2 + HASH_LEN,
            )
        } else {
            (None, 2)
        };
        let destination = raw.get(at..at + HASH_LEN)?.try_into().ok()?;
        let context = *raw.get(at + HASH_LEN)?;
        let data = raw.get(at + HASH_LEN + 1..)?.to_vec();
        if data.is_empty() {
            return None;
        }
        Some(Self {
            raw: raw.to_vec(),
            packet_type: match flags & 0x03 {
                0 => PacketType::Data,
                1 => PacketType::Announce,
                2 => PacketType::LinkRequest,
                _ => PacketType::Proof,
            },
            plain: (flags >> 2) & 0x03 == PLAIN,
            context_flag: flags & CONTEXT_FLAG != 0,
            hops: raw[1],
            transport_id,
            destination,
            context,
            data,
        })
    }

    /// The packet's hash, as Reticulum computes it to drop duplicates: the
    /// same however many hops it took and whichever header it carries.
    pub fn hash(&self) -> [u8; 32] {
        let rest = if self.transport_id.is_some() {
            &self.raw[2 + HASH_LEN..]
        } else {
            &self.raw[2..]
        };
        sha256(&[&[self.raw[0] & 0x0F], rest])
    }
}

/// A validated announce of a single destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announce {
    pub destination: [u8; HASH_LEN],
    pub public_key: [u8; PUBLIC_KEY_LEN],
    pub name_hash: [u8; NAME_HASH_LEN],
    /// Hops as the packet arrived.
    pub hops: u8,
    /// The transport node it came through, which packets to it go through
    /// too; none when it came from the node itself.
    pub transport_id: Option<[u8; HASH_LEN]>,
    /// When it was made, by its sender's clock.
    pub emitted: u64,
}

impl Announce {
    /// The announce a packet carries, if its signature verifies and its
    /// destination hash matches its key and name.
    pub fn validate(packet: &Packet) -> Option<Self> {
        if packet.packet_type != PacketType::Announce || packet.plain {
            return None;
        }
        let data = &packet.data;
        let public_key: [u8; PUBLIC_KEY_LEN] = data.get(..PUBLIC_KEY_LEN)?.try_into().ok()?;
        let name_hash: [u8; NAME_HASH_LEN] = data.get(64..74)?.try_into().ok()?;
        let random_hash = data.get(74..84)?;
        let ratchet_len = if packet.context_flag { 32 } else { 0 };
        let ratchet = data.get(84..84 + ratchet_len)?;
        let signature_at = 84 + ratchet_len;
        let signature = data.get(signature_at..signature_at + SIGNATURE_LEN)?;
        let app_data = &data[signature_at + SIGNATURE_LEN..];
        let signed = [
            &packet.destination[..],
            &public_key,
            &name_hash,
            random_hash,
            ratchet,
            app_data,
        ]
        .concat();
        let key = VerifyingKey::from_bytes(public_key[32..].try_into().ok()?).ok()?;
        key.verify(&signed, &Signature::from_slice(signature).ok()?)
            .ok()?;
        let expected = truncated(sha256(&[&name_hash, &identity_hash(&public_key)]));
        if expected != packet.destination {
            return None;
        }
        let mut emitted = [0; 8];
        emitted[3..].copy_from_slice(&random_hash[5..]);
        Some(Self {
            destination: packet.destination,
            public_key,
            name_hash,
            hops: packet.hops,
            transport_id: packet.transport_id,
            emitted: u64::from_be_bytes(emitted),
        })
    }
}

/// An announce of `identity`'s CarryChat destination made at `timestamp`,
/// which must be later than its last one, with no app data. As an answer to
/// a path request it says so.
pub fn announce(identity: &Identity, timestamp: u64, path_response: bool) -> Vec<u8> {
    let destination = identity.destination();
    let public_key = identity.public_key();
    let name_hash = name_hash(NAME);
    let mut random_hash = [0; RANDOM_HASH_LEN];
    OsRng.fill_bytes(&mut random_hash[..5]);
    random_hash[5..].copy_from_slice(&timestamp.to_be_bytes()[3..]);
    let signed = [&destination[..], &public_key, &name_hash, &random_hash].concat();
    let signature = identity.signing.sign(&signed);
    let context = if path_response {
        CONTEXT_PATH_RESPONSE
    } else {
        CONTEXT_NONE
    };
    let mut raw = vec![(SINGLE << 2) | ANNOUNCE, 0];
    raw.extend_from_slice(&destination);
    raw.push(context);
    raw.extend_from_slice(&public_key);
    raw.extend_from_slice(&name_hash);
    raw.extend_from_slice(&random_hash);
    raw.extend_from_slice(&signature.to_bytes());
    raw
}

/// A data packet carrying `plaintext` to the destination `to` announced,
/// encrypted to it. It goes through the transport node the announce came
/// through, if any, as the second header names it: a transport node drops a
/// packet for a destination further away that does not.
pub fn data_packet(to: &Announce, plaintext: &[u8]) -> Vec<u8> {
    let mut raw = match to.transport_id {
        Some(transport_id) => {
            let mut raw = vec![HEADER_2 | TRANSPORT | (SINGLE << 2) | DATA, 0];
            raw.extend_from_slice(&transport_id);
            raw
        }
        None => vec![(SINGLE << 2) | DATA, 0],
    };
    raw.extend_from_slice(&to.destination);
    raw.push(CONTEXT_NONE);
    raw.extend(encrypt(&to.public_key, plaintext));
    raw
}

/// A request for the path to `destination`, which a transport node that
/// knows it answers with the destination's announce. `tag` must be new.
pub fn path_request(destination: &[u8; HASH_LEN], tag: &[u8; 16]) -> Vec<u8> {
    let mut raw = vec![(PLAIN << 2) | DATA, 0];
    raw.extend_from_slice(&plain_destination_hash(PATH_REQUEST));
    raw.push(CONTEXT_NONE);
    raw.extend_from_slice(destination);
    raw.extend_from_slice(tag);
    raw
}

/// The destination a path request asks for, if `packet` is one.
pub fn requested_path(packet: &Packet) -> Option<[u8; HASH_LEN]> {
    if packet.packet_type != PacketType::Data
        || !packet.plain
        || packet.destination != plain_destination_hash(PATH_REQUEST)
    {
        return None;
    }
    packet.data.get(..HASH_LEN)?.try_into().ok()
}

/// `raw` framed for a TCP interface.
pub fn frame(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 8);
    out.push(FLAG);
    for &byte in raw {
        if byte == FLAG || byte == ESCAPE {
            out.push(ESCAPE);
            out.push(byte ^ ESCAPE_MASK);
        } else {
            out.push(byte);
        }
    }
    out.push(FLAG);
    out
}

/// Takes the frames out of a TCP interface's byte stream.
#[derive(Default)]
pub struct Deframer {
    buffer: Vec<u8>,
}

impl Deframer {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > MAX_FRAME_LEN * 2 {
            self.buffer.clear();
        }
    }

    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            let start = self.buffer.iter().position(|&byte| byte == FLAG)?;
            let end = start
                + 1
                + self.buffer[start + 1..]
                    .iter()
                    .position(|&byte| byte == FLAG)?;
            let escaped = self.buffer[start + 1..end].to_vec();
            // The closing flag may open the next frame.
            self.buffer.drain(..end);
            let mut raw = Vec::with_capacity(escaped.len());
            let mut bytes = escaped.into_iter();
            while let Some(byte) = bytes.next() {
                if byte == ESCAPE {
                    raw.push(bytes.next().unwrap_or(0) ^ ESCAPE_MASK);
                } else {
                    raw.push(byte);
                }
            }
            if raw.len() >= MIN_FRAME_LEN && raw.len() <= MAX_FRAME_LEN {
                return Some(raw);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        data_encoding::HEXLOWER.encode(bytes)
    }

    fn identity(seed: u8) -> Identity {
        Identity::from_secret(&[seed; 64])
    }

    #[test]
    fn names_hash_as_reticulum_hashes_them() {
        // Computed from RNS 1.5.5's definitions.
        assert_eq!(hex(&name_hash(NAME)), "87af905a5b8b7d7a8da6");
        assert_eq!(
            hex(&plain_destination_hash(PATH_REQUEST)),
            "6b9f66014d9853faab220fba47d02761"
        );
    }

    #[test]
    fn a_packet_encrypted_to_an_identity_opens_only_for_it() {
        let alice = identity(1);
        let bob = identity(2);
        for len in [0, 1, 15, 16, 17, MTU] {
            let plaintext: Vec<u8> = (0..len).map(|byte| byte as u8).collect();
            let data = encrypt(&alice.public_key(), &plaintext);
            assert_eq!(alice.decrypt(&data), Some(plaintext.clone()));
            assert_eq!(bob.decrypt(&data), None);
        }
        let mut data = encrypt(&alice.public_key(), b"frame");
        let last = data.len() - 1;
        data[last] ^= 1;
        assert_eq!(alice.decrypt(&data), None);
    }

    #[test]
    fn an_announce_validates_and_names_its_sender() {
        let alice = identity(3);
        let raw = announce(&alice, 1_800_000_000, false);
        let packet = Packet::parse(&raw).unwrap();
        let announce = Announce::validate(&packet).unwrap();
        assert_eq!(announce.destination, alice.destination());
        assert_eq!(announce.public_key, alice.public_key());
        assert_eq!(announce.name_hash, name_hash(NAME));
        assert_eq!(announce.emitted, 1_800_000_000);
        assert_eq!(announce.transport_id, None);

        // Changing a byte of the key spoils the signature or the hash.
        let mut forged = raw.clone();
        forged[19 + 40] ^= 1;
        assert_eq!(Announce::validate(&Packet::parse(&forged).unwrap()), None);
    }

    #[test]
    fn a_packet_through_a_transport_node_names_it() {
        let alice = identity(4);
        let mut announced =
            Announce::validate(&Packet::parse(&announce(&alice, 1, false)).unwrap()).unwrap();
        let plain = Packet::parse(&data_packet(&announced, b"hello")).unwrap();
        assert_eq!(plain.transport_id, None);
        assert_eq!(plain.destination, alice.destination());
        announced.transport_id = Some([9; HASH_LEN]);
        let raw = data_packet(&announced, b"hello");
        assert_eq!(raw[0], 0x50);
        let through = Packet::parse(&raw).unwrap();
        assert_eq!(through.transport_id, Some([9; HASH_LEN]));
        assert_eq!(through.destination, alice.destination());
        assert_eq!(alice.decrypt(&through.data).as_deref(), Some(&b"hello"[..]));

        // The hash ignores the hops and the transport node.
        let mut stripped = vec![raw[0] & 0x0F, 3];
        stripped.extend_from_slice(&raw[2 + HASH_LEN..]);
        assert_eq!(Packet::parse(&stripped).unwrap().hash(), through.hash());
    }

    #[test]
    fn frames_survive_stuffing_and_short_ones_are_skipped() {
        let raw: Vec<u8> = (0..40).map(|byte| [FLAG, ESCAPE, 1][byte % 3]).collect();
        let mut stream = vec![FLAG, FLAG];
        stream.extend(frame(&raw));
        stream.extend(frame(&[1, 2, 3]));
        stream.extend(frame(&raw));
        let mut deframer = Deframer::default();
        deframer.push(&stream[..7]);
        assert_eq!(deframer.next_frame(), None);
        deframer.push(&stream[7..]);
        assert_eq!(deframer.next_frame(), Some(raw.clone()));
        assert_eq!(deframer.next_frame(), Some(raw));
        assert_eq!(deframer.next_frame(), None);
    }

    #[test]
    fn a_path_request_names_its_destination() {
        let raw = path_request(&[7; HASH_LEN], &[8; 16]);
        assert_eq!(raw[0], 0x08);
        let packet = Packet::parse(&raw).unwrap();
        assert_eq!(requested_path(&packet), Some([7; HASH_LEN]));
    }
}

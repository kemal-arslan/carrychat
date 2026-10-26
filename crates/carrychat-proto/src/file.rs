//! Files (§5.9): a photo or a voice message is cut into chunks sealed with a
//! key of its own, and offered in an ordinary message that carries the key.

use core::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rand_core::CryptoRngCore;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::bundle::{Bundle, BundleId, Kind, Message, Tombstone, HEADER_LEN};
use crate::pair::{AckSecret, RecipientTag};
use crate::{hmac_sha256, sha256, truncate, Error, Result};

/// Bytes of a file in each chunk; the last one may hold fewer.
pub const CHUNK_LEN: usize = 4000;
pub const MAX_FILE_LEN: usize = 1 << 20;
pub const MAX_MIME_LEN: usize = 100;

const SALT: &[u8] = b"carrychat/v1/file";

/// What a file is, and the key to its chunks: the body of a file message.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct FileOffer {
    #[zeroize(skip)]
    pub mime: String,
    pub size: u32,
    pub key: [u8; 32],
    pub hash: [u8; 32],
}

impl FileOffer {
    pub fn to_bytes(&self) -> Vec<u8> {
        [
            &[self.mime.len() as u8][..],
            self.mime.as_bytes(),
            &self.size.to_be_bytes(),
            &self.key,
            &self.hash,
        ]
        .concat()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let (&mime_len, rest) = bytes.split_first().ok_or(Error::Truncated)?;
        let mime_len = mime_len as usize;
        match rest.len().cmp(&(mime_len + 4 + 32 + 32)) {
            core::cmp::Ordering::Less => return Err(Error::Truncated),
            core::cmp::Ordering::Greater => return Err(Error::TooLong),
            core::cmp::Ordering::Equal => {}
        }
        let mime = String::from_utf8(rest[..mime_len].to_vec())
            .map_err(|_| Error::Invalid("media type"))?;
        let offer = Self {
            mime,
            size: u32::from_be_bytes(truncate(&rest[mime_len..])),
            key: truncate(&rest[mime_len + 4..]),
            hash: truncate(&rest[mime_len + 36..]),
        };
        offer.check()?;
        Ok(offer)
    }

    /// How many chunks the file is cut into.
    pub fn chunks(&self) -> u32 {
        (self.size as usize).div_ceil(CHUNK_LEN) as u32
    }

    pub fn keys(&self) -> FileKeys {
        FileKeys::derive(&self.key)
    }

    fn check(&self) -> Result<()> {
        if self.mime.is_empty() || self.mime.len() > MAX_MIME_LEN {
            return Err(Error::Invalid("media type"));
        }
        if self.size == 0 || self.size as usize > MAX_FILE_LEN {
            return Err(Error::Invalid("file size"));
        }
        Ok(())
    }
}

impl fmt::Debug for FileOffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileOffer")
            .field("mime", &self.mime)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl Message {
    /// Offers a file whose chunks travel on their own (§5.9).
    pub fn file(offer: &FileOffer, created_at: u32) -> Self {
        Self {
            kind: Kind::File,
            created_at,
            body: offer.to_bytes(),
        }
    }
}

/// The keys of a file's chunks, from the key in its offer.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct FileKeys {
    enc: [u8; 32],
    tag: [u8; 32],
    ack: [u8; 32],
}

impl FileKeys {
    pub fn derive(key: &[u8; 32]) -> Self {
        let hkdf = Hkdf::<Sha256>::new(Some(SALT), key);
        let mut keys = Self {
            enc: [0; 32],
            tag: [0; 32],
            ack: [0; 32],
        };
        for (info, out) in [
            (&b"enc"[..], &mut keys.enc),
            (b"tag", &mut keys.tag),
            (b"ack", &mut keys.ack),
        ] {
            hkdf.expand(info, out)
                .expect("32 bytes is a valid HKDF-SHA256 output length");
        }
        keys
    }

    /// The tag of chunk `index`.
    pub fn tag(&self, index: u32) -> RecipientTag {
        truncate(&hmac_sha256(&self.tag, &[&index.to_be_bytes()]))
    }

    fn ack_secret(&self, nonce: &[u8; 24]) -> AckSecret {
        truncate(&hmac_sha256(&self.ack, &[nonce]))
    }

    pub fn seal_chunk(
        &self,
        index: u32,
        chunk: &[u8],
        expires_at: u32,
        hop_limit: u8,
        nonce: [u8; 24],
    ) -> Result<Bundle> {
        if chunk.is_empty() || chunk.len() > CHUNK_LEN {
            return Err(Error::Invalid("chunk length"));
        }
        let mut bundle = Bundle {
            hop_limit,
            expires_at,
            id: BundleId::from_ack_secret(&self.ack_secret(&nonce)),
            tag: self.tag(index),
            nonce,
            ciphertext: Vec::new(),
        };
        bundle.ciphertext = XChaCha20Poly1305::new((&self.enc).into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: chunk,
                    aad: &bundle.aad(),
                },
            )
            .map_err(|_| Error::Invalid("chunk"))?;
        Ok(bundle)
    }

    /// Opens a chunk whose tag is one of this file's. The tag, part of the
    /// associated data, ties it to its place in the file.
    pub fn open_chunk(&self, bundle: &Bundle) -> Result<Vec<u8>> {
        let chunk = XChaCha20Poly1305::new((&self.enc).into())
            .decrypt(
                XNonce::from_slice(&bundle.nonce),
                Payload {
                    msg: &bundle.ciphertext,
                    aad: &bundle.aad(),
                },
            )
            .map_err(|_| Error::Decrypt)?;
        if BundleId::from_ack_secret(&self.ack_secret(&bundle.nonce)) != bundle.id {
            return Err(Error::Invalid("bundle id"));
        }
        Ok(chunk)
    }

    /// Proof that a chunk arrived, for carriers to drop it (§6).
    pub fn tombstone(&self, bundle: &Bundle) -> Tombstone {
        Tombstone {
            id: bundle.id,
            ack_secret: self.ack_secret(&bundle.nonce),
        }
    }
}

impl fmt::Debug for FileKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileKeys").finish_non_exhaustive()
    }
}

/// Seals a file with a fresh key: its offer and its chunks.
pub fn seal_file(
    mime: &str,
    data: &[u8],
    expires_at: u32,
    hop_limit: u8,
    rng: &mut impl CryptoRngCore,
) -> Result<(FileOffer, Vec<Bundle>)> {
    let mut key = [0; 32];
    rng.fill_bytes(&mut key);
    seal_file_with(mime, data, expires_at, hop_limit, key, || {
        let mut nonce = [0; 24];
        rng.fill_bytes(&mut nonce);
        nonce
    })
}

/// Seals with a caller-chosen key and nonces, for test vectors. Use [`seal_file`].
pub fn seal_file_with(
    mime: &str,
    data: &[u8],
    expires_at: u32,
    hop_limit: u8,
    key: [u8; 32],
    mut nonce: impl FnMut() -> [u8; 24],
) -> Result<(FileOffer, Vec<Bundle>)> {
    let offer = FileOffer {
        mime: mime.to_owned(),
        size: u32::try_from(data.len()).map_err(|_| Error::TooLong)?,
        key,
        hash: sha256(&[data]),
    };
    offer.check()?;
    let keys = offer.keys();
    let chunks = data
        .chunks(CHUNK_LEN)
        .enumerate()
        .map(|(index, chunk)| keys.seal_chunk(index as u32, chunk, expires_at, hop_limit, nonce()))
        .collect::<Result<_>>()?;
    Ok((offer, chunks))
}

/// Joins a file's chunks, in order, and checks them against its offer.
pub fn assemble(offer: &FileOffer, chunks: &[Vec<u8>]) -> Result<Vec<u8>> {
    if chunks.len() != offer.chunks() as usize {
        return Err(Error::Invalid("number of chunks"));
    }
    let data = chunks.concat();
    if data.len() != offer.size as usize || sha256(&[&data]) != offer.hash {
        return Err(Error::Invalid("file hash"));
    }
    Ok(data)
}

/// Bytes a chunk bundle adds around its share of the file.
pub const CHUNK_OVERHEAD: usize = HEADER_LEN + 16;

#[cfg(test)]
mod tests {
    use rand_core::OsRng;

    use super::*;
    use crate::bundle::MAX_BUNDLE_LEN;
    use crate::frame::Frame;

    const NOW: u32 = 1_790_000_000;

    fn photo(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    fn open_all(offer: &FileOffer, chunks: &[Bundle]) -> Result<Vec<u8>> {
        let keys = offer.keys();
        let mut opened = vec![Vec::new(); chunks.len()];
        for bundle in chunks {
            let index = (0..offer.chunks())
                .find(|&i| keys.tag(i) == bundle.tag)
                .ok_or(Error::Invalid("tag"))?;
            opened[index as usize] = keys.open_chunk(bundle)?;
        }
        assemble(offer, &opened)
    }

    #[test]
    fn a_file_comes_back_from_its_chunks_in_any_order() {
        let data = photo(9_000);
        let (offer, mut chunks) = seal_file("image/jpeg", &data, NOW + 60, 8, &mut OsRng).unwrap();
        assert_eq!(offer.chunks(), 3);
        assert_eq!(chunks.len(), 3);
        chunks.reverse();
        let offer = FileOffer::from_bytes(&offer.to_bytes()).unwrap();
        assert_eq!(open_all(&offer, &chunks).unwrap(), data);
        // Every chunk fits a bundle and travels as a chunk frame.
        for bundle in &chunks {
            assert!(bundle.to_bytes().len() <= MAX_BUNDLE_LEN);
            let frame = Frame::from_bytes(&Frame::Chunk(bundle.clone()).to_bytes()).unwrap();
            assert_eq!(frame, Frame::Chunk(bundle.clone()));
        }
    }

    #[test]
    fn a_one_byte_tail_still_parses() {
        let data = photo(CHUNK_LEN + 1);
        let (offer, chunks) = seal_file("audio/mp4", &data, NOW + 60, 8, &mut OsRng).unwrap();
        let tail = Bundle::from_bytes(&chunks[1].to_bytes()).unwrap();
        assert_eq!(
            offer.keys().open_chunk(&tail).unwrap(),
            vec![data[CHUNK_LEN]]
        );
    }

    #[test]
    fn chunks_cannot_be_swapped_changed_or_opened_without_the_key() {
        let data = photo(8_000);
        let (offer, chunks) = seal_file("image/jpeg", &data, NOW + 60, 8, &mut OsRng).unwrap();
        let keys = offer.keys();
        // Moving a chunk to the other place changes its tag, which it is sealed with.
        let mut moved = chunks[0].clone();
        moved.tag = keys.tag(1);
        assert!(keys.open_chunk(&moved).is_err());
        for index in (0..chunks[0].to_bytes().len()).filter(|&i| i != 1) {
            let mut bytes = chunks[0].to_bytes();
            bytes[index] ^= 0x01;
            if let Ok(bundle) = Bundle::from_bytes(&bytes) {
                assert!(keys.open_chunk(&bundle).is_err(), "byte {index}");
            }
        }
        assert!(FileKeys::derive(&[0; 32]).open_chunk(&chunks[0]).is_err());

        let mut wrong = offer.clone();
        wrong.hash[0] ^= 1;
        assert!(open_all(&wrong, &chunks).is_err());
        assert!(open_all(&offer, &chunks[..1]).is_err());
    }

    #[test]
    fn tombstones_prove_a_chunk_arrived() {
        let (offer, chunks) =
            seal_file("image/jpeg", &photo(100), NOW + 60, 8, &mut OsRng).unwrap();
        let tombstone = offer.keys().tombstone(&chunks[0]);
        assert!(tombstone.verify());
        assert_eq!(tombstone.id, chunks[0].id);
    }

    #[test]
    fn offers_are_checked() {
        assert!(seal_file("image/jpeg", &[], NOW + 60, 8, &mut OsRng).is_err());
        assert!(seal_file(
            "image/jpeg",
            &photo(MAX_FILE_LEN + 1),
            NOW + 60,
            8,
            &mut OsRng
        )
        .is_err());
        assert!(seal_file("", &photo(10), NOW + 60, 8, &mut OsRng).is_err());
        let (offer, _) = seal_file("image/jpeg", &photo(10), NOW + 60, 8, &mut OsRng).unwrap();
        let bytes = offer.to_bytes();
        assert!(FileOffer::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        assert!(FileOffer::from_bytes(&[bytes.as_slice(), &[0]].concat()).is_err());
        assert_eq!(Message::file(&offer, NOW).kind, Kind::File);
    }
}

//! Reference implementation of the Kervan v0 wire protocol: identities,
//! contact cards, pair keys, sealed bundles, tombstones and link frames.
//!
//! Nothing here does I/O. Transports and storage build on top of it. The
//! normative description is `docs/PROTOCOL.md`; section numbers in comments
//! refer to it.

pub mod bundle;
pub mod error;
pub mod frame;
pub mod identity;
pub mod intro;
pub mod pair;
pub mod ratchet;

pub use error::{Error, Result};

/// Protocol version carried in cards and bundles.
pub const VERSION: u8 = 0;

/// Length of a tag epoch in seconds.
pub const DAY_SECS: u32 = 86_400;

/// The tag epoch a unix time falls in (§2).
pub fn day_of(unix_secs: u32) -> u32 {
    unix_secs / DAY_SECS
}

pub(crate) fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

pub(crate) fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    let mut mac =
        <Hmac<sha2::Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

pub(crate) fn truncate<const N: usize>(bytes: &[u8]) -> [u8; N] {
    bytes[..N].try_into().expect("input is at least N bytes")
}

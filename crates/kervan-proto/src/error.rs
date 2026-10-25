#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("unsupported version {0}")]
    UnsupportedVersion(u8),
    #[error("input is truncated")]
    Truncated,
    #[error("input is too long")]
    TooLong,
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("signature does not verify")]
    BadSignature,
    #[error("decryption failed")]
    Decrypt,
    #[error("peer key is not contributory")]
    WeakKey,
}

pub type Result<T> = core::result::Result<T, Error>;

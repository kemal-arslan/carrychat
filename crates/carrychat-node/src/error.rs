#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Protocol(#[from] carrychat_proto::Error),
    #[error("database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(&'static str),
    #[error("key store: {0}")]
    KeyStore(String),
    #[cfg(feature = "bluetooth")]
    #[error("Bluetooth: {0}")]
    Bluetooth(#[from] btleplug::Error),
}

pub type Result<T> = core::result::Result<T, Error>;

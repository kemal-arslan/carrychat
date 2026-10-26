#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Protocol(#[from] kervan_proto::Error),
    #[error("database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(&'static str),
    #[cfg(feature = "bluetooth")]
    #[error("Bluetooth: {0}")]
    Bluetooth(#[from] btleplug::Error),
}

pub type Result<T> = core::result::Result<T, Error>;

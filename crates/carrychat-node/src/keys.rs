//! The key a node's data is encrypted with on the device (§10). Everything
//! the node keeps, its identity, contacts, sessions and messages, is one
//! SQLCipher database; its key is kept apart from it, by the operating
//! system's key store where there is one, so that a copy of the data folder
//! alone opens nothing.

use std::fmt;
use std::path::Path;

use data_encoding::HEXLOWER;
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

use crate::{Error, Result};

/// The file [`KeyFile`] keeps the key in.
const KEY_FILE: &str = "carrychat.key";

/// A 256-bit key for a node's database.
#[derive(Clone, PartialEq, Eq)]
pub struct DataKey(Zeroizing<[u8; 32]>);

impl DataKey {
    pub fn random() -> Self {
        let mut key = Zeroizing::new([0; 32]);
        OsRng.fill_bytes(key.as_mut());
        Self(key)
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        Some(Self(Zeroizing::new(bytes.try_into().ok()?)))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The key as SQLCipher takes it raw, with no key derivation: `x'…'`.
    pub(crate) fn sql(&self) -> Zeroizing<String> {
        Zeroizing::new(format!("x'{}'", HEXLOWER.encode(self.as_bytes())))
    }
}

impl fmt::Debug for DataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DataKey(…)")
    }
}

/// Where a device keeps the key to the data in each data folder.
pub trait KeyStore: Send + Sync {
    /// The key to the data in `dir`, if one is kept.
    fn load(&self, dir: &Path) -> Result<Option<DataKey>>;
    /// Keeps `key` as the key to the data in `dir`.
    fn save(&self, dir: &Path, key: &DataKey) -> Result<()>;
    /// Forgets the key to the data in `dir`, if one is kept.
    fn delete(&self, dir: &Path) -> Result<()>;
}

/// Keeps the key in a file in the data folder, which only its owner can
/// read. Against whoever can read the folder that protects nothing: it is
/// for tests, and for devices with no key store, such as a relay with no
/// screen.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyFile;

impl KeyStore for KeyFile {
    fn load(&self, dir: &Path) -> Result<Option<DataKey>> {
        match std::fs::read(dir.join(KEY_FILE)) {
            Ok(bytes) => DataKey::from_bytes(&Zeroizing::new(bytes))
                .map(Some)
                .ok_or(Error::Invalid("the data key file is damaged")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn save(&self, dir: &Path, key: &DataKey) -> Result<()> {
        // Written whole under another name, then put in place.
        let path = dir.join(KEY_FILE);
        let new = dir.join(format!("{KEY_FILE}.new"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&new)?;
        std::io::Write::write_all(&mut file, key.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&new, &path)?;
        Ok(())
    }

    fn delete(&self, dir: &Path) -> Result<()> {
        match std::fs::remove_file(dir.join(KEY_FILE)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// The first key store, or the second where the first cannot keep a key:
/// a key the first holds is used, then one the second holds.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fallback<First, Second>(pub First, pub Second);

impl<First: KeyStore, Second: KeyStore> KeyStore for Fallback<First, Second> {
    fn load(&self, dir: &Path) -> Result<Option<DataKey>> {
        match self.0.load(dir) {
            Ok(Some(key)) => Ok(Some(key)),
            Ok(None) => self.1.load(dir),
            // A key store that cannot be reached may still hold the key:
            // with none in the second either, that is the error.
            Err(error) => self.1.load(dir)?.map(Some).ok_or(error),
        }
    }

    fn save(&self, dir: &Path, key: &DataKey) -> Result<()> {
        match self.0.save(dir, key) {
            Ok(()) => Ok(()),
            Err(error) => {
                eprintln!("No key store ({error}): the data key is kept next to the data.");
                self.1.save(dir, key)
            }
        }
    }

    fn delete(&self, dir: &Path) -> Result<()> {
        let first = self.0.delete(dir);
        self.1.delete(dir)?;
        first
    }
}

/// The operating system's key store: macOS Keychain, Windows Credential
/// Manager, or the Secret Service on Linux. A key is kept under the data
/// folder's full path, so a folder moved elsewhere needs its key moved too.
#[cfg(feature = "keychain")]
#[derive(Clone, Copy, Debug, Default)]
pub struct Keychain;

#[cfg(feature = "keychain")]
impl Keychain {
    const SERVICE: &'static str = "CarryChat";

    fn entry(dir: &Path) -> Result<keyring::Entry> {
        let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_owned());
        keyring::Entry::new(Self::SERVICE, &dir.to_string_lossy()).map_err(key_store)
    }
}

#[cfg(feature = "keychain")]
impl KeyStore for Keychain {
    fn load(&self, dir: &Path) -> Result<Option<DataKey>> {
        match Self::entry(dir)?.get_secret() {
            Ok(secret) => DataKey::from_bytes(&Zeroizing::new(secret))
                .map(Some)
                .ok_or(Error::Invalid("the key store holds a damaged data key")),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(key_store(error)),
        }
    }

    fn save(&self, dir: &Path, key: &DataKey) -> Result<()> {
        Self::entry(dir)?
            .set_secret(key.as_bytes())
            .map_err(key_store)
    }

    fn delete(&self, dir: &Path) -> Result<()> {
        match Self::entry(dir)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(key_store(error)),
        }
    }
}

#[cfg(feature = "keychain")]
fn key_store(error: keyring::Error) -> Error {
    Error::KeyStore(error.to_string())
}

/// The key for data in `dir` that is not encrypted yet, or not there yet:
/// the one `keys` keeps, or else a new one, kept in `keys` and read back to
/// make sure it was. A key store that cannot be read counts as holding
/// none, since a key it may hold opens nothing yet.
pub(crate) fn load_or_create(keys: &dyn KeyStore, dir: &Path) -> Result<DataKey> {
    match keys.load(dir) {
        Ok(Some(key)) => return Ok(key),
        Ok(None) => {}
        Err(error) => eprintln!("The data key could not be read ({error}): making a new one."),
    }
    let key = DataKey::random();
    keys.save(dir, &key)?;
    match keys.load(dir)? {
        Some(kept) if kept == key => Ok(key),
        _ => Err(Error::Invalid("the key store did not keep the data key")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A key store that cannot be reached.
    struct Unreachable;

    impl KeyStore for Unreachable {
        fn load(&self, _: &Path) -> Result<Option<DataKey>> {
            Err(Error::KeyStore("unreachable".to_owned()))
        }
        fn save(&self, _: &Path, _: &DataKey) -> Result<()> {
            Err(Error::KeyStore("unreachable".to_owned()))
        }
        fn delete(&self, _: &Path) -> Result<()> {
            Err(Error::KeyStore("unreachable".to_owned()))
        }
    }

    /// A key store in memory.
    #[derive(Default)]
    struct Memory(Mutex<Option<DataKey>>);

    impl KeyStore for Memory {
        fn load(&self, _: &Path) -> Result<Option<DataKey>> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, _: &Path, key: &DataKey) -> Result<()> {
            *self.0.lock().unwrap() = Some(key.clone());
            Ok(())
        }
        fn delete(&self, _: &Path) -> Result<()> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    #[test]
    fn a_key_file_keeps_the_key_for_its_owner_only() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(KeyFile.load(dir.path()).unwrap(), None);
        let key = load_or_create(&KeyFile, dir.path()).unwrap();
        assert_eq!(KeyFile.load(dir.path()).unwrap(), Some(key.clone()));
        assert_eq!(load_or_create(&KeyFile, dir.path()).unwrap(), key);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        KeyFile.delete(dir.path()).unwrap();
        assert_eq!(KeyFile.load(dir.path()).unwrap(), None);
    }

    #[test]
    fn a_fallback_uses_the_second_store_only_where_the_first_cannot() {
        let dir = tempfile::TempDir::new().unwrap();
        // With the first unreachable, the key goes to the second.
        let keys = Fallback(Unreachable, KeyFile);
        let key = load_or_create(&keys, dir.path()).unwrap();
        assert_eq!(KeyFile.load(dir.path()).unwrap(), Some(key.clone()));
        assert_eq!(keys.load(dir.path()).unwrap(), Some(key));

        // A key the first holds is used, and the second is left alone.
        let other = tempfile::TempDir::new().unwrap();
        let keys = Fallback(Memory::default(), KeyFile);
        let key = load_or_create(&keys, other.path()).unwrap();
        assert_eq!(keys.0.load(other.path()).unwrap(), Some(key));
        assert_eq!(KeyFile.load(other.path()).unwrap(), None);

        // Neither reachable nor holding a key: an error, not a new key.
        let empty = tempfile::TempDir::new().unwrap();
        assert!(Fallback(Unreachable, Unreachable)
            .load(empty.path())
            .is_err());
    }

    #[test]
    fn a_key_never_shows_in_debug_output() {
        assert_eq!(format!("{:?}", DataKey::random()), "DataKey(…)");
    }
}

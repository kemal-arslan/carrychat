//! A node's data is encrypted on the device (§10): one SQLCipher database,
//! whose key is kept apart from it.

use std::path::Path;

use carrychat_node::{DataKey, KeyFile, KeyStore, Node};
use rusqlite::Connection;
use tempfile::TempDir;

const DATABASE: &str = "carrychat.db";

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_default()
}

/// Every file the database is in, as one.
fn database_bytes(dir: &Path) -> Vec<u8> {
    ["", "-wal", "-shm"]
        .iter()
        .flat_map(|suffix| read(&dir.join(format!("{DATABASE}{suffix}"))))
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Makes `dir`'s database plain again, as from before encryption, with its
/// key gone. With `crashed`, its last change is still in its write-ahead
/// log, as a program that ended without closing it leaves it.
fn decrypt(dir: &Path, crashed: bool) {
    let key = KeyFile.load(dir).unwrap().unwrap();
    let plain = dir.join("plain.db");
    {
        let conn = Connection::open(dir.join(DATABASE)).unwrap();
        conn.execute_batch(&format!(
            "PRAGMA key = \"x'{}'\";",
            data_encoding::HEXLOWER.encode(key.as_bytes())
        ))
        .unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        conn.execute_batch(&format!(
            "ATTACH DATABASE '{}' AS plain KEY ''; SELECT sqlcipher_export('plain');
             PRAGMA plain.user_version = {version}; DETACH DATABASE plain;",
            plain.display()
        ))
        .unwrap();
    }
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.join(format!("{DATABASE}{suffix}")));
    }
    std::fs::rename(&plain, dir.join(DATABASE)).unwrap();
    KeyFile.delete(dir).unwrap();
    if crashed {
        let conn = Connection::open(dir.join(DATABASE)).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;
             DELETE FROM contacts;",
        )
        .unwrap();
        // Copied while open: the change is only in the log.
        let crashed_dir = dir.join("crashed");
        std::fs::create_dir(&crashed_dir).unwrap();
        for suffix in ["", "-wal", "-shm"] {
            let name = format!("{DATABASE}{suffix}");
            std::fs::copy(dir.join(&name), crashed_dir.join(&name)).unwrap();
        }
        drop(conn);
        for suffix in ["", "-wal", "-shm"] {
            let name = format!("{DATABASE}{suffix}");
            let _ = std::fs::remove_file(dir.join(&name));
            std::fs::rename(crashed_dir.join(&name), dir.join(&name)).unwrap();
        }
        std::fs::remove_dir(&crashed_dir).unwrap();
    }
}

#[test]
fn a_node_keeps_its_data_encrypted() {
    let dir = TempDir::new().unwrap();
    let node = Node::create(dir.path(), "Ayşe Gizli").unwrap();
    let card = node.profile().card;
    drop(node);

    let bytes = database_bytes(dir.path());
    assert!(!bytes.starts_with(b"SQLite format 3"));
    assert!(!contains(&bytes, "Ayşe Gizli".as_bytes()));
    // Without its key, SQLite cannot read it.
    let conn = Connection::open(dir.path().join(DATABASE)).unwrap();
    assert!(conn
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| row
            .get::<_, i64>(0))
        .is_err());

    let node = Node::open(dir.path()).unwrap().unwrap();
    assert_eq!(node.profile().card, card);
}

#[test]
fn data_from_before_encryption_is_encrypted_and_kept() {
    for crashed in [false, true] {
        let dir = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        let node = Node::create(dir.path(), "Ayşe").unwrap();
        let deniz = Node::create(other.path(), "Deniz").unwrap();
        node.add_contact(&deniz.profile().card).unwrap();
        let card = node.profile().card;
        drop(node);
        decrypt(dir.path(), crashed);
        assert!(database_bytes(dir.path()).starts_with(b"SQLite format 3"));
        assert_eq!(KeyFile.load(dir.path()).unwrap(), None);

        // The change still in the log is kept as well.
        let node = Node::open(dir.path()).unwrap().unwrap();
        assert_eq!(node.profile().card, card, "crashed: {crashed}");
        let contacts: Vec<String> = node
            .contacts()
            .unwrap()
            .into_iter()
            .map(|contact| contact.name)
            .collect();
        let expected: &[&str] = if crashed { &[] } else { &["Deniz"] };
        assert_eq!(contacts, expected, "crashed: {crashed}");
        drop(node);

        let bytes = database_bytes(dir.path());
        assert!(!bytes.starts_with(b"SQLite format 3"));
        assert!(!contains(&bytes, card.as_bytes()));
        assert!(KeyFile.load(dir.path()).unwrap().is_some());
        assert!(!dir.path().join("carrychat.db.encrypting").exists());
        // It opens again as it is, with the same identity.
        let node = Node::open(dir.path()).unwrap().unwrap();
        assert_eq!(node.profile().card, card);
    }
}

#[test]
fn data_without_its_key_is_left_as_it_is() {
    let dir = TempDir::new().unwrap();
    drop(Node::create(dir.path(), "Ayşe").unwrap());
    let before = database_bytes(dir.path());

    // The key is gone: nothing opens, and nothing is made in its place.
    KeyFile.delete(dir.path()).unwrap();
    assert!(Node::open(dir.path()).is_err());
    assert!(Node::create(dir.path(), "Başka").is_err());
    assert_eq!(KeyFile.load(dir.path()).unwrap(), None);
    assert_eq!(database_bytes(dir.path()), before);

    // Another key opens nothing either.
    KeyFile.save(dir.path(), &DataKey::random()).unwrap();
    assert!(Node::open(dir.path()).is_err());
    assert_eq!(database_bytes(dir.path()), before);
}

#[test]
fn an_interrupted_encryption_is_done_again() {
    let dir = TempDir::new().unwrap();
    drop(Node::create(dir.path(), "Ayşe").unwrap());
    decrypt(dir.path(), false);
    // A half-written copy from a try that ended midway.
    std::fs::write(dir.path().join("carrychat.db.encrypting"), b"half").unwrap();

    let node = Node::open(dir.path()).unwrap().unwrap();
    assert_eq!(node.profile().name, "Ayşe");
    assert!(!dir.path().join("carrychat.db.encrypting").exists());
}

#[test]
fn erasing_forgets_the_key_too() {
    let dir = TempDir::new().unwrap();
    drop(Node::create(dir.path(), "Ayşe").unwrap());
    carrychat_node::erase(dir.path()).unwrap();
    assert_eq!(KeyFile.load(dir.path()).unwrap(), None);
    assert!(Node::open(dir.path()).unwrap().is_none());
    // A new identity can be made there, with a new key.
    let node = Node::create(dir.path(), "Yeni").unwrap();
    assert_eq!(node.profile().name, "Yeni");
}

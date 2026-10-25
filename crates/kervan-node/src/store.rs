//! Everything a node keeps on disk, in one SQLite file.

use std::path::Path;

use kervan_proto::bundle::{Bundle, BundleId, Tombstone};
use kervan_proto::identity::ContactCard;
use rusqlite::{params, Connection, OptionalExtension};

use crate::{MessageView, Result};

/// Bundles carried for others, at most. Own bundles do not count.
pub const MAX_CARRIED_BUNDLES: usize = 2000;
pub const MAX_CARRIED_BYTES: usize = 8 * 1024 * 1024;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS contacts (
    id INTEGER PRIMARY KEY,
    card BLOB NOT NULL,
    dh_key BLOB NOT NULL UNIQUE,
    added_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY,
    contact_id INTEGER NOT NULL REFERENCES contacts (id),
    outgoing INTEGER NOT NULL,
    bundle_id BLOB NOT NULL,
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    received_at INTEGER,
    delivered_at INTEGER,
    UNIQUE (bundle_id, outgoing)
);
CREATE INDEX IF NOT EXISTS messages_by_contact ON messages (contact_id, created_at);
CREATE TABLE IF NOT EXISTS bundles (
    id BLOB PRIMARY KEY,
    bytes BLOB NOT NULL,
    expires_at INTEGER NOT NULL,
    hop_limit INTEGER NOT NULL,
    own INTEGER NOT NULL,
    stored_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS tombstones (
    id BLOB PRIMARY KEY,
    ack_secret BLOB NOT NULL,
    keep_until INTEGER NOT NULL
);
";

/// Changes to the schema since the first release, applied in order. The
/// index of the last one applied is kept in `PRAGMA user_version`.
const MIGRATIONS: &[&str] = &[
    // Requests and introductions (§5.5).
    "ALTER TABLE contacts ADD COLUMN request INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE contacts ADD COLUMN introduced INTEGER NOT NULL DEFAULT 0;
     CREATE TABLE blocked (dh_key BLOB PRIMARY KEY);",
    // Read receipts: when we read an incoming message, or they read ours.
    "ALTER TABLE messages ADD COLUMN read_at INTEGER;",
];

pub struct StoredContact {
    pub id: i64,
    pub card: ContactCard,
    pub request: bool,
    pub introduced: bool,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        let applied: usize = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        for (index, migration) in MIGRATIONS.iter().enumerate().skip(applied) {
            conn.execute_batch(&format!(
                "BEGIN; {migration} PRAGMA user_version = {}; COMMIT;",
                index + 1
            ))?;
        }
        Ok(Self { conn })
    }

    pub fn meta(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?, ?)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Adds a contact, or returns the id it already has.
    /// Adds a contact, or returns the id it already has. Adding someone as a
    /// contact also accepts them if they were a request.
    pub fn insert_contact(&self, card: &ContactCard, now: u32, request: bool) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO contacts (card, dh_key, added_at, request) VALUES (?, ?, ?, ?)
             ON CONFLICT (dh_key) DO UPDATE SET
                 card = excluded.card,
                 request = MIN(contacts.request, excluded.request)",
            params![card.to_bytes(), card.dh_key().as_bytes(), now, request],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM contacts WHERE dh_key = ?",
            [card.dh_key().as_bytes()],
            |row| row.get(0),
        )?)
    }

    pub fn contacts(&self) -> Result<Vec<StoredContact>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, card, request, introduced FROM contacts ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })?;
        let mut contacts = Vec::new();
        for row in rows {
            let (id, card, request, introduced) = row?;
            contacts.push(StoredContact {
                id,
                card: ContactCard::from_bytes(&card)?,
                request,
                introduced,
            });
        }
        Ok(contacts)
    }

    pub fn accept_contact(&self, id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE contacts SET request = 0 WHERE id = ?", [id])?;
        Ok(())
    }

    pub fn set_introduced(&self, id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE contacts SET introduced = 1 WHERE id = ?", [id])?;
        Ok(())
    }

    /// Deletes a contact with its messages.
    pub fn delete_contact(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM messages WHERE contact_id = ?", [id])?;
        self.conn
            .execute("DELETE FROM contacts WHERE id = ?", [id])?;
        Ok(())
    }

    pub fn block(&self, dh_key: &[u8; 32]) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO blocked (dh_key) VALUES (?)",
            [dh_key],
        )?;
        Ok(())
    }

    pub fn unblock(&self, dh_key: &[u8; 32]) -> Result<()> {
        self.conn
            .execute("DELETE FROM blocked WHERE dh_key = ?", [dh_key])?;
        Ok(())
    }

    pub fn is_blocked(&self, dh_key: &[u8; 32]) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM blocked WHERE dh_key = ?", [dh_key], |_| {
                Ok(())
            })
            .optional()?
            .is_some())
    }

    /// Records a message. Returns `None` if it is already recorded.
    pub fn insert_message(
        &self,
        contact_id: i64,
        outgoing: bool,
        bundle_id: BundleId,
        body: &str,
        created_at: u32,
        received_at: Option<u32>,
    ) -> Result<Option<MessageView>> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO messages
             (contact_id, outgoing, bundle_id, body, created_at, received_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                contact_id,
                outgoing,
                bundle_id.0,
                body,
                created_at,
                received_at
            ],
        )?;
        Ok((inserted == 1).then(|| MessageView {
            id: self.conn.last_insert_rowid(),
            contact_id,
            outgoing,
            body: body.to_owned(),
            created_at,
            delivered: false,
            read: false,
        }))
    }

    pub fn messages(&self, contact_id: i64) -> Result<Vec<MessageView>> {
        let mut statement = self.conn.prepare(
            "SELECT id, outgoing, body, created_at, delivered_at IS NOT NULL, read_at IS NOT NULL
             FROM messages WHERE contact_id = ? ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([contact_id], |row| {
            Ok(MessageView {
                id: row.get(0)?,
                contact_id,
                outgoing: row.get(1)?,
                body: row.get(2)?,
                created_at: row.get(3)?,
                delivered: row.get(4)?,
                read: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn last_message(&self, contact_id: i64) -> Result<Option<(String, u32)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT body, created_at FROM messages WHERE contact_id = ?
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                [contact_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub fn has_message(&self, id: BundleId) -> Result<bool> {
        self.exists("SELECT 1 FROM messages WHERE bundle_id = ?", id)
    }

    /// Marks an outgoing message delivered. Returns its contact and message id
    /// the first time.
    pub fn mark_delivered(&self, id: BundleId, now: u32) -> Result<Option<(i64, i64)>> {
        Ok(self
            .conn
            .query_row(
                "UPDATE messages SET delivered_at = ?
                 WHERE bundle_id = ? AND outgoing = 1 AND delivered_at IS NULL
                 RETURNING contact_id, id",
                params![now, id.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub fn has_bundle(&self, id: BundleId) -> Result<bool> {
        self.exists("SELECT 1 FROM bundles WHERE id = ?", id)
    }

    pub fn insert_bundle(&self, bundle: &Bundle, own: bool, now: u32) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO bundles (id, bytes, expires_at, hop_limit, own, stored_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                bundle.id.0,
                bundle.to_bytes(),
                bundle.expires_at,
                bundle.hop_limit,
                own,
                now
            ],
        )?;
        if !own {
            self.enforce_quota()?;
        }
        Ok(())
    }

    pub fn delete_bundle(&self, id: BundleId) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM bundles WHERE id = ?", [id.0])?
            == 1)
    }

    pub fn bundle(&self, id: BundleId) -> Result<Option<Bundle>> {
        let bytes: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT bytes FROM bundles WHERE id = ?", [id.0], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(bytes.map(|bytes| Bundle::from_bytes(&bytes)).transpose()?)
    }

    /// Bundles that may still be passed on: not expired, hop limit left.
    pub fn forwardable_bundles(&self, now: u32) -> Result<Vec<Bundle>> {
        self.bundles_where("expires_at > ?1 AND hop_limit > 0", now)
    }

    /// Own bundles still waiting for a receipt.
    pub fn own_bundles(&self, now: u32) -> Result<Vec<Bundle>> {
        self.bundles_where("expires_at > ?1 AND own = 1", now)
    }

    /// The contact and creation time of one of our own messages.
    pub fn outgoing_message(&self, bundle_id: BundleId) -> Result<Option<(i64, u32)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT contact_id, created_at FROM messages WHERE bundle_id = ? AND outgoing = 1",
                [bundle_id.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// Tombstones of the most recent bundles delivered to us.
    pub fn received_tombstones(&self, limit: usize) -> Result<Vec<Tombstone>> {
        let mut statement = self.conn.prepare(
            "SELECT t.id, t.ack_secret FROM tombstones t
             JOIN messages m ON m.bundle_id = t.id AND m.outgoing = 0
             ORDER BY m.received_at DESC LIMIT ?",
        )?;
        let rows = statement.query_map([limit], |row| {
            let id: [u8; 8] = row.get(0)?;
            let ack_secret: [u8; 16] = row.get(1)?;
            Ok(Tombstone {
                id: BundleId(id),
                ack_secret,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn carried_bundles(&self, now: u32) -> Result<Vec<Bundle>> {
        self.bundles_where("expires_at > ?1 AND own = 0", now)
    }

    /// Own and carried bundle counts.
    /// Own messages not delivered yet, and bundles carried for others.
    pub fn counts(&self) -> Result<(usize, usize)> {
        Ok(self.conn.query_row(
            "SELECT
                 (SELECT COUNT(*) FROM messages m JOIN bundles b ON b.id = m.bundle_id
                  WHERE m.outgoing = 1 AND m.delivered_at IS NULL),
                 (SELECT COUNT(*) FROM bundles WHERE own = 0)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }

    pub fn unread_count(&self, contact_id: i64) -> Result<usize> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE contact_id = ? AND outgoing = 0 AND read_at IS NULL",
            [contact_id],
            |row| row.get(0),
        )?)
    }

    /// Marks a contact's messages to us read. Returns the bundle ids that were
    /// unread.
    pub fn mark_incoming_read(&self, contact_id: i64, now: u32) -> Result<Vec<BundleId>> {
        let mut statement = self.conn.prepare(
            "UPDATE messages SET read_at = ?
             WHERE contact_id = ? AND outgoing = 0 AND read_at IS NULL
             RETURNING bundle_id",
        )?;
        let rows = statement.query_map(params![now, contact_id], |row| {
            Ok(BundleId(row.get::<_, [u8; 8]>(0)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Marks our messages to a contact read by them. Returns the message ids
    /// that were not marked yet.
    pub fn mark_read_by_peer(
        &self,
        contact_id: i64,
        ids: &[BundleId],
        now: u32,
    ) -> Result<Vec<i64>> {
        let mut marked = Vec::new();
        for id in ids {
            let message: Option<i64> = self
                .conn
                .query_row(
                    "UPDATE messages SET read_at = ?
                     WHERE contact_id = ? AND outgoing = 1 AND bundle_id = ? AND read_at IS NULL
                     RETURNING id",
                    params![now, contact_id, id.0],
                    |row| row.get(0),
                )
                .optional()?;
            marked.extend(message);
        }
        Ok(marked)
    }

    pub fn update_contact_card(&self, id: i64, card: &ContactCard) -> Result<()> {
        self.conn.execute(
            "UPDATE contacts SET card = ? WHERE id = ?",
            params![card.to_bytes(), id],
        )?;
        Ok(())
    }

    pub fn has_tombstone(&self, id: BundleId) -> Result<bool> {
        self.exists("SELECT 1 FROM tombstones WHERE id = ?", id)
    }

    pub fn insert_tombstone(&self, tombstone: &Tombstone, keep_until: u32) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO tombstones (id, ack_secret, keep_until) VALUES (?, ?, ?)",
            params![tombstone.id.0, tombstone.ack_secret, keep_until],
        )?;
        Ok(())
    }

    pub fn tombstones(&self) -> Result<Vec<Tombstone>> {
        let mut statement = self.conn.prepare("SELECT id, ack_secret FROM tombstones")?;
        let rows = statement.query_map([], |row| {
            let id: [u8; 8] = row.get(0)?;
            let ack_secret: [u8; 16] = row.get(1)?;
            Ok(Tombstone {
                id: BundleId(id),
                ack_secret,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Deletes expired bundles and tombstones that are no longer needed.
    pub fn prune(&self, now: u32) -> Result<()> {
        self.conn
            .execute("DELETE FROM bundles WHERE expires_at <= ?", [now])?;
        self.conn
            .execute("DELETE FROM tombstones WHERE keep_until <= ?", [now])?;
        Ok(())
    }

    /// Evicts carried bundles closest to expiry until the quota holds (§8).
    fn enforce_quota(&self) -> Result<()> {
        loop {
            let (count, bytes): (usize, usize) = self.conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(LENGTH(bytes)), 0) FROM bundles WHERE own = 0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if count <= MAX_CARRIED_BUNDLES && bytes <= MAX_CARRIED_BYTES {
                return Ok(());
            }
            self.conn.execute(
                "DELETE FROM bundles WHERE id = (
                     SELECT id FROM bundles WHERE own = 0 ORDER BY expires_at LIMIT 1
                 )",
                [],
            )?;
        }
    }

    fn bundles_where(&self, condition: &str, now: u32) -> Result<Vec<Bundle>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT bytes FROM bundles WHERE {condition} ORDER BY stored_at"
        ))?;
        let rows = statement.query_map([now], |row| row.get::<_, Vec<u8>>(0))?;
        let mut bundles = Vec::new();
        for bytes in rows {
            bundles.push(Bundle::from_bytes(&bytes?)?);
        }
        Ok(bundles)
    }

    fn exists(&self, sql: &str, id: BundleId) -> Result<bool> {
        Ok(self
            .conn
            .query_row(sql, [id.0], |_| Ok(()))
            .optional()?
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kervan_proto::identity::Identity;

    #[test]
    fn a_database_from_the_first_release_is_migrated() {
        let dir = std::env::temp_dir().join(format!("kervan-migration-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.db");
        let _ = std::fs::remove_file(&path);
        let card = Identity::generate("Deniz").unwrap().card();
        {
            let old = Connection::open(&path).unwrap();
            old.execute_batch(SCHEMA).unwrap();
            old.execute(
                "INSERT INTO contacts (card, dh_key, added_at) VALUES (?, ?, 1)",
                params![card.to_bytes(), card.dh_key().as_bytes()],
            )
            .unwrap();
        }

        let store = Store::open(&path).unwrap();
        let contacts = store.contacts().unwrap();
        assert_eq!(contacts.len(), 1);
        assert!(!contacts[0].request);
        assert!(!contacts[0].introduced);
        assert!(!store.is_blocked(card.dh_key().as_bytes()).unwrap());
        drop(store);

        // Opening again applies nothing twice.
        Store::open(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

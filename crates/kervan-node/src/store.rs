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
    pub fn insert_contact(&self, card: &ContactCard, now: u32) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO contacts (card, dh_key, added_at) VALUES (?, ?, ?)
             ON CONFLICT (dh_key) DO UPDATE SET card = excluded.card",
            params![card.to_bytes(), card.dh_key().as_bytes(), now],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM contacts WHERE dh_key = ?",
            [card.dh_key().as_bytes()],
            |row| row.get(0),
        )?)
    }

    pub fn contacts(&self) -> Result<Vec<(i64, ContactCard)>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, card FROM contacts ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut contacts = Vec::new();
        for row in rows {
            let (id, bytes) = row?;
            contacts.push((id, ContactCard::from_bytes(&bytes)?));
        }
        Ok(contacts)
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
        }))
    }

    pub fn messages(&self, contact_id: i64) -> Result<Vec<MessageView>> {
        let mut statement = self.conn.prepare(
            "SELECT id, outgoing, body, created_at, delivered_at IS NOT NULL FROM messages
             WHERE contact_id = ? ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([contact_id], |row| {
            Ok(MessageView {
                id: row.get(0)?,
                contact_id,
                outgoing: row.get(1)?,
                body: row.get(2)?,
                created_at: row.get(3)?,
                delivered: row.get(4)?,
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

    pub fn carried_bundles(&self, now: u32) -> Result<Vec<Bundle>> {
        self.bundles_where("expires_at > ?1 AND own = 0", now)
    }

    /// Own and carried bundle counts.
    pub fn bundle_counts(&self) -> Result<(usize, usize)> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(SUM(own = 1), 0), COALESCE(SUM(own = 0), 0) FROM bundles",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
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

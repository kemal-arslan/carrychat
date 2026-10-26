//! Everything a node keeps on disk, in one SQLite file.

use std::path::Path;

use carrychat_proto::bundle::{Bundle, BundleId, Tombstone};
use carrychat_proto::identity::ContactCard;
use carrychat_proto::ratchet::SkippedKey;
use rusqlite::{params, Connection, OptionalExtension};

use crate::journey::{Journey, Step, Via};
use crate::{MessageView, Result};

mod channels;
mod files;
mod groups;

pub use channels::{StoredChannel, StoredPost};
pub use files::{FileTable, StoredFile};
pub use groups::{StoredGroup, StoredGroupMessage};

/// Bundles carried for others, at most. Own bundles do not count.
pub const MAX_CARRIED_BUNDLES: usize = 2000;
pub const MAX_CARRIED_BYTES: usize = 8 * 1024 * 1024;
/// File chunks carried for others, at most, apart from messages (§5.9).
pub const MAX_CARRIED_CHUNKS: usize = 16_000;
pub const MAX_CARRIED_CHUNK_BYTES: usize = 64 * 1024 * 1024;

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
    // Ratchet sessions (§5.6) and the keys of messages that have not arrived.
    "CREATE TABLE sessions (contact_id INTEGER PRIMARY KEY, state BLOB NOT NULL);
     CREATE TABLE skipped_keys (
         contact_id INTEGER NOT NULL,
         ratchet BLOB NOT NULL,
         n INTEGER NOT NULL,
         key BLOB NOT NULL,
         stored_at INTEGER NOT NULL,
         PRIMARY KEY (contact_id, ratchet, n)
     );",
    // Groups (§5.7): members, sender chains and messages. Group peers are
    // members we know only through a group, kept as hidden contacts.
    "ALTER TABLE contacts ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;
     CREATE TABLE groups (
         id INTEGER PRIMARY KEY,
         group_id BLOB NOT NULL UNIQUE,
         name TEXT NOT NULL,
         admin BLOB NOT NULL,
         version INTEGER NOT NULL,
         member INTEGER NOT NULL,
         created_at INTEGER NOT NULL
     );
     CREATE TABLE group_members (
         group_ref INTEGER NOT NULL REFERENCES groups (id),
         position INTEGER NOT NULL,
         dh_key BLOB NOT NULL,
         card BLOB NOT NULL,
         PRIMARY KEY (group_ref, dh_key)
     );
     CREATE TABLE sender_keys (
         id INTEGER PRIMARY KEY,
         group_id BLOB NOT NULL,
         sender BLOB NOT NULL,
         chain_id BLOB NOT NULL UNIQUE,
         state BLOB NOT NULL,
         own INTEGER NOT NULL,
         created_at INTEGER NOT NULL,
         retired_at INTEGER
     );
     CREATE TABLE group_skipped (
         chain INTEGER NOT NULL,
         iteration INTEGER NOT NULL,
         key BLOB NOT NULL,
         stored_at INTEGER NOT NULL,
         PRIMARY KEY (chain, iteration)
     );
     CREATE TABLE group_messages (
         id INTEGER PRIMARY KEY,
         group_ref INTEGER NOT NULL REFERENCES groups (id),
         sender BLOB,
         bundle_id BLOB NOT NULL UNIQUE,
         body TEXT NOT NULL,
         created_at INTEGER NOT NULL,
         received_at INTEGER,
         read_at INTEGER
     );
     CREATE INDEX group_messages_by_group ON group_messages (group_ref, created_at);",
    // Public channels (§5.8): their keys, posts, and authors we muted.
    "CREATE TABLE channels (
         id INTEGER PRIMARY KEY,
         name TEXT NOT NULL,
         channel_id BLOB NOT NULL UNIQUE,
         keys BLOB NOT NULL,
         has_password INTEGER NOT NULL,
         nick TEXT NOT NULL,
         joined_at INTEGER NOT NULL
     );
     CREATE TABLE channel_posts (
         id INTEGER PRIMARY KEY,
         channel_ref INTEGER NOT NULL REFERENCES channels (id),
         author BLOB NOT NULL,
         outgoing INTEGER NOT NULL,
         nick TEXT NOT NULL,
         card INTEGER NOT NULL,
         bundle_id BLOB NOT NULL UNIQUE,
         body TEXT NOT NULL,
         created_at INTEGER NOT NULL,
         received_at INTEGER,
         read_at INTEGER
     );
     CREATE INDEX channel_posts_by_channel ON channel_posts (channel_ref, created_at);
     CREATE TABLE channel_muted (
         channel_ref INTEGER NOT NULL REFERENCES channels (id),
         author BLOB NOT NULL,
         PRIMARY KEY (channel_ref, author)
     );",
    // Files (§5.9): chunk bundles apart from messages, offers, and the
    // chunks of files still arriving.
    "ALTER TABLE bundles ADD COLUMN chunk INTEGER NOT NULL DEFAULT 0;
     CREATE TABLE files (
         id INTEGER PRIMARY KEY,
         offer BLOB NOT NULL UNIQUE,
         pair INTEGER NOT NULL,
         data BLOB,
         expires_at INTEGER NOT NULL
     );
     CREATE TABLE file_chunks (
         file_ref INTEGER NOT NULL REFERENCES files (id),
         idx INTEGER NOT NULL,
         data BLOB NOT NULL,
         PRIMARY KEY (file_ref, idx)
     );
     ALTER TABLE messages ADD COLUMN file_ref INTEGER;
     ALTER TABLE group_messages ADD COLUMN file_ref INTEGER;
     ALTER TABLE channel_posts ADD COLUMN file_ref INTEGER;",
    // Whether a carried bundle has gone up to the internet relays.
    "ALTER TABLE bundles ADD COLUMN uploaded INTEGER NOT NULL DEFAULT 0;",
    // Where our own messages went: each link, and the device or relay that
    // took a message over it, the first time.
    "CREATE TABLE handoffs (
         bundle_id BLOB NOT NULL,
         via TEXT NOT NULL,
         peer INTEGER NOT NULL,
         at INTEGER NOT NULL,
         PRIMARY KEY (bundle_id, via, peer)
     );",
];

/// Skipped keys kept per contact, at most; the oldest go first.
const MAX_SKIPPED_KEYS: usize = 2000;
/// How long a skipped key is kept: a bundle's longest life and a day.
const SKIPPED_KEY_LIFETIME: u32 = 8 * 86_400;

pub struct StoredContact {
    pub id: i64,
    pub card: ContactCard,
    pub request: bool,
    pub introduced: bool,
    /// Known only as a member of a group (§5.7).
    pub hidden: bool,
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
        // Deleted keys must not linger in free pages (§5.6).
        conn.pragma_update(None, "secure_delete", "ON")?;
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

    /// Starts a change that is kept only if [`Store::commit`] follows, and
    /// undone by [`Store::rollback`]. Changes nest.
    pub fn begin(&self) -> Result<()> {
        self.conn.execute_batch("SAVEPOINT change")?;
        Ok(())
    }

    pub fn commit(&self) -> Result<()> {
        self.conn.execute_batch("RELEASE change")?;
        Ok(())
    }

    pub fn rollback(&self) {
        let _ = self
            .conn
            .execute_batch("ROLLBACK TO change; RELEASE change");
    }

    /// Runs `write` so that it is kept whole or not at all, inside any change
    /// already under way.
    pub(crate) fn whole<T>(&self, write: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        self.begin()?;
        match write(&self.conn) {
            Ok(value) => {
                self.commit()?;
                Ok(value)
            }
            Err(error) => {
                self.rollback();
                Err(error)
            }
        }
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

    /// Adds a contact, or returns the id it already has. Adding someone as a
    /// contact also accepts them if they were a request, and shows them if
    /// they were a group peer.
    pub fn insert_contact(&self, card: &ContactCard, now: u32, request: bool) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO contacts (card, dh_key, added_at, request) VALUES (?, ?, ?, ?)
             ON CONFLICT (dh_key) DO UPDATE SET
                 card = excluded.card,
                 request = MIN(contacts.request, excluded.request),
                 hidden = 0",
            params![card.to_bytes(), card.dh_key().as_bytes(), now, request],
        )?;
        self.contact_id(card)
    }

    /// Adds a group peer: someone we know only through a group, who has our
    /// card through it. Returns the id it already has if it is known.
    pub fn insert_peer(&self, card: &ContactCard, now: u32) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO contacts (card, dh_key, added_at, introduced, hidden) VALUES (?, ?, ?, 1, 1)
             ON CONFLICT (dh_key) DO NOTHING",
            params![card.to_bytes(), card.dh_key().as_bytes(), now],
        )?;
        self.contact_id(card)
    }

    /// Turns a group peer that wrote to us directly into a request.
    pub fn reveal_peer(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE contacts SET hidden = 0, request = 1 WHERE id = ? AND hidden = 1",
            [id],
        )?;
        Ok(())
    }

    fn contact_id(&self, card: &ContactCard) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT id FROM contacts WHERE dh_key = ?",
            [card.dh_key().as_bytes()],
            |row| row.get(0),
        )?)
    }

    pub fn contacts(&self) -> Result<Vec<StoredContact>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, card, request, introduced, hidden FROM contacts ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })?;
        let mut contacts = Vec::new();
        for row in rows {
            let (id, card, request, introduced, hidden) = row?;
            contacts.push(StoredContact {
                id,
                card: ContactCard::from_bytes(&card)?,
                request,
                introduced,
                hidden,
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

    pub fn session(&self, contact_id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row(
                "SELECT state FROM sessions WHERE contact_id = ?",
                [contact_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn save_session(&self, contact_id: i64, state: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (contact_id, state) VALUES (?, ?)
             ON CONFLICT (contact_id) DO UPDATE SET state = excluded.state",
            params![contact_id, state],
        )?;
        Ok(())
    }

    pub fn skipped_key(
        &self,
        contact_id: i64,
        ratchet: &[u8; 32],
        n: u16,
    ) -> Result<Option<[u8; 32]>> {
        Ok(self
            .conn
            .query_row(
                "SELECT key FROM skipped_keys WHERE contact_id = ? AND ratchet = ? AND n = ?",
                params![contact_id, ratchet, n],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn delete_skipped_key(&self, contact_id: i64, ratchet: &[u8; 32], n: u16) -> Result<()> {
        self.conn.execute(
            "DELETE FROM skipped_keys WHERE contact_id = ? AND ratchet = ? AND n = ?",
            params![contact_id, ratchet, n],
        )?;
        Ok(())
    }

    pub fn store_skipped_keys(&self, contact_id: i64, keys: &[SkippedKey], now: u32) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        self.whole(|tx| {
            for key in keys {
                tx.execute(
                    "INSERT OR REPLACE INTO skipped_keys (contact_id, ratchet, n, key, stored_at)
                 VALUES (?, ?, ?, ?, ?)",
                    params![contact_id, key.ratchet, key.n, key.key, now],
                )?;
            }
            tx.execute(
                "DELETE FROM skipped_keys WHERE contact_id = ?1 AND rowid NOT IN (
                 SELECT rowid FROM skipped_keys WHERE contact_id = ?1
                 ORDER BY stored_at DESC, n DESC LIMIT ?2
             )",
                params![contact_id, MAX_SKIPPED_KEYS],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// Deletes a contact with its messages and session.
    pub fn delete_contact(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM skipped_keys WHERE contact_id = ?", [id])?;
        self.conn
            .execute("DELETE FROM sessions WHERE contact_id = ?", [id])?;
        self.conn.execute(
            "DELETE FROM handoffs WHERE bundle_id IN
                 (SELECT bundle_id FROM messages WHERE contact_id = ? AND outgoing = 1)",
            [id],
        )?;
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
            file: None,
            via: Vec::new(),
        }))
    }

    pub fn messages(&self, contact_id: i64) -> Result<Vec<MessageView>> {
        let mut statement = self.conn.prepare(
            "SELECT id, outgoing, body, created_at, delivered_at IS NOT NULL, read_at IS NOT NULL,
                 (SELECT GROUP_CONCAT(via) FROM
                     (SELECT via FROM handoffs WHERE handoffs.bundle_id = messages.bundle_id
                      GROUP BY via ORDER BY MIN(at), MIN(rowid)))
             FROM messages WHERE contact_id = ? ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([contact_id], |row| {
            let via: Option<String> = row.get(6)?;
            Ok(MessageView {
                id: row.get(0)?,
                contact_id,
                outgoing: row.get(1)?,
                body: row.get(2)?,
                created_at: row.get(3)?,
                delivered: row.get(4)?,
                read: row.get(5)?,
                file: None,
                via: via
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(Via::parse)
                    .collect(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Notes that one of our messages, still on its way, went to `peer` over
    /// `via`. Returns its contact and message id when `via` is new for it.
    pub fn record_handoff(
        &self,
        id: BundleId,
        via: Via,
        peer: u64,
        now: u32,
    ) -> Result<Option<(i64, i64)>> {
        let Some((contact_id, message_id)) = self
            .conn
            .query_row(
                "SELECT contact_id, id FROM messages
                 WHERE bundle_id = ? AND outgoing = 1 AND delivered_at IS NULL",
                [id.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
        else {
            return Ok(None);
        };
        let new_way = !self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM handoffs WHERE bundle_id = ? AND via = ?)",
            params![id.0, via.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        // Session ids are random 64-bit numbers; SQLite keeps them signed.
        self.conn.execute(
            "INSERT OR IGNORE INTO handoffs (bundle_id, via, peer, at) VALUES (?, ?, ?, ?)",
            params![id.0, via.as_str(), peer as i64, now],
        )?;
        Ok(new_way.then_some((contact_id, message_id)))
    }

    /// The way one of our messages went, from writing to reading.
    pub fn journey(&self, message_id: i64) -> Result<Option<Journey>> {
        let Some((bundle_id, created_at, delivered_at, read_at)) = self
            .conn
            .query_row(
                "SELECT bundle_id, created_at, delivered_at, read_at FROM messages
                 WHERE id = ? AND outgoing = 1",
                [message_id],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        let mut statement = self.conn.prepare(
            "SELECT via, COUNT(*), MIN(at) FROM handoffs WHERE bundle_id = ?
             GROUP BY via ORDER BY MIN(at), MIN(rowid)",
        )?;
        let steps = statement
            .query_map([bundle_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, usize>(1)?,
                    row.get(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter_map(|(via, count, at)| {
                Some(Step {
                    via: Via::parse(&via)?,
                    count,
                    at,
                })
            })
            .collect();
        Ok(Some(Journey {
            created_at,
            delivered_at,
            read_at,
            steps,
        }))
    }

    /// Our messages to a contact that have not arrived yet.
    pub fn waiting_count(&self, contact_id: i64) -> Result<usize> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM messages
             WHERE contact_id = ? AND outgoing = 1 AND delivered_at IS NULL",
            [contact_id],
            |row| row.get(0),
        )?)
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
        self.insert_bundle_of(bundle, own, false, now)
    }

    /// Keeps a file chunk (§5.9), under the quota for chunks.
    pub fn insert_chunk_bundle(&self, bundle: &Bundle, own: bool, now: u32) -> Result<()> {
        self.insert_bundle_of(bundle, own, true, now)
    }

    fn insert_bundle_of(&self, bundle: &Bundle, own: bool, chunk: bool, now: u32) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO bundles (id, bytes, expires_at, hop_limit, own, stored_at, chunk)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                bundle.id.0,
                bundle.to_bytes(),
                bundle.expires_at,
                bundle.hop_limit,
                own,
                now,
                chunk
            ],
        )?;
        if !own {
            self.enforce_quota(chunk)?;
        }
        Ok(())
    }

    /// Carried bundles not yet given to the internet relays, which may still
    /// travel on.
    pub fn bundles_to_upload(&self, now: u32) -> Result<Vec<Bundle>> {
        self.bundles_where(
            "expires_at > ?1 AND own = 0 AND uploaded = 0 AND hop_limit > 0",
            now,
        )
    }

    pub fn mark_uploaded(&self, id: BundleId) -> Result<()> {
        self.conn
            .execute("UPDATE bundles SET uploaded = 1 WHERE id = ?", [id.0])?;
        Ok(())
    }

    /// Whether a stored bundle is a file chunk, which travels in chunk frames.
    pub fn is_chunk(&self, id: BundleId) -> Result<bool> {
        self.exists("SELECT 1 FROM bundles WHERE id = ? AND chunk = 1", id)
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

    /// Bundles carried for others, file chunks apart.
    pub fn carried_bundles(&self, now: u32) -> Result<Vec<Bundle>> {
        self.bundles_where("expires_at > ?1 AND own = 0 AND chunk = 0", now)
    }

    pub fn carried_chunks(&self, now: u32) -> Result<Vec<Bundle>> {
        self.bundles_where("expires_at > ?1 AND own = 0 AND chunk = 1", now)
    }

    /// Own and carried bundle counts.
    /// Own messages not delivered yet, and bundles carried for others.
    pub fn counts(&self) -> Result<(usize, usize)> {
        Ok(self.conn.query_row(
            "SELECT
                 (SELECT COUNT(*) FROM messages m JOIN bundles b ON b.id = m.bundle_id
                  WHERE m.outgoing = 1 AND m.delivered_at IS NULL),
                 (SELECT COUNT(*) FROM bundles WHERE own = 0 AND chunk = 0)",
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
        self.conn.execute(
            "DELETE FROM skipped_keys WHERE stored_at <= ?",
            [now.saturating_sub(SKIPPED_KEY_LIFETIME)],
        )?;
        self.prune_files(now)?;
        self.prune_groups(now)
    }

    /// Evicts carried bundles closest to expiry until the quota holds (§8).
    /// Messages and file chunks have quotas of their own.
    fn enforce_quota(&self, chunk: bool) -> Result<()> {
        let (max_count, max_bytes) = if chunk {
            (MAX_CARRIED_CHUNKS, MAX_CARRIED_CHUNK_BYTES)
        } else {
            (MAX_CARRIED_BUNDLES, MAX_CARRIED_BYTES)
        };
        loop {
            let (count, bytes): (usize, usize) = self.conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(LENGTH(bytes)), 0) FROM bundles
                 WHERE own = 0 AND chunk = ?",
                [chunk],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if count <= max_count && bytes <= max_bytes {
                return Ok(());
            }
            self.conn.execute(
                "DELETE FROM bundles WHERE id = (
                     SELECT id FROM bundles WHERE own = 0 AND chunk = ? ORDER BY expires_at LIMIT 1
                 )",
                [chunk],
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
    use carrychat_proto::identity::Identity;

    #[test]
    fn a_database_from_the_first_release_is_migrated() {
        let dir = std::env::temp_dir().join(format!("carrychat-migration-{}", std::process::id()));
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

    #[test]
    fn skipped_keys_are_capped_expire_and_go_with_the_contact() {
        let store = Store::open_in_memory().unwrap();
        let card = Identity::generate("Deniz").unwrap().card();
        let id = store.insert_contact(&card, 1, false).unwrap();
        let ratchet = [7; 32];
        let keys: Vec<SkippedKey> = (0..=MAX_SKIPPED_KEYS as u16)
            .map(|n| SkippedKey {
                ratchet,
                n,
                key: [n as u8; 32],
            })
            .collect();
        store.store_skipped_keys(id, &keys, 100).unwrap();
        // One over the cap: the oldest goes.
        assert_eq!(store.skipped_key(id, &ratchet, 0).unwrap(), None);
        assert_eq!(store.skipped_key(id, &ratchet, 5).unwrap(), Some([5; 32]));
        store.delete_skipped_key(id, &ratchet, 5).unwrap();
        assert_eq!(store.skipped_key(id, &ratchet, 5).unwrap(), None);

        store.prune(100 + SKIPPED_KEY_LIFETIME - 1).unwrap();
        assert!(store.skipped_key(id, &ratchet, 6).unwrap().is_some());
        store.prune(100 + SKIPPED_KEY_LIFETIME).unwrap();
        assert_eq!(store.skipped_key(id, &ratchet, 6).unwrap(), None);

        store.store_skipped_keys(id, &keys[..1], 200).unwrap();
        store.save_session(id, &[1, 2, 3]).unwrap();
        assert_eq!(store.session(id).unwrap(), Some(vec![1, 2, 3]));
        store.delete_contact(id).unwrap();
        assert_eq!(store.session(id).unwrap(), None);
        assert_eq!(store.skipped_key(id, &ratchet, 0).unwrap(), None);
    }
}

//! Public channels in the store (§5.8): their keys, posts and muted authors.

use carrychat_proto::bundle::BundleId;
use carrychat_proto::channel::ChannelKeys;
use rusqlite::{params, OptionalExtension};

use super::Store;
use crate::Result;

pub struct StoredChannel {
    pub id: i64,
    /// The normalized name, without `#`.
    pub name: String,
    pub keys: ChannelKeys,
    pub has_password: bool,
    /// The name we post under.
    pub nick: String,
}

pub struct StoredPost {
    pub id: i64,
    pub author: [u8; 32],
    pub outgoing: bool,
    pub nick: String,
    /// Whether the body is the author's card rather than text.
    pub card: bool,
    pub body: String,
    pub created_at: u32,
    pub read: bool,
}

impl Store {
    pub fn channels(&self) -> Result<Vec<StoredChannel>> {
        self.channels_where("1", rusqlite::types::Value::Null)
    }

    pub fn channel(&self, id: i64) -> Result<Option<StoredChannel>> {
        Ok(self
            .channels_where("id = ?1", rusqlite::types::Value::Integer(id))?
            .into_iter()
            .next())
    }

    fn channels_where(
        &self,
        condition: &str,
        value: rusqlite::types::Value,
    ) -> Result<Vec<StoredChannel>> {
        let sql = format!(
            "SELECT id, name, keys, has_password, nick FROM channels
             WHERE {condition} OR ?1 IS NULL ORDER BY id"
        );
        let rows: Vec<(i64, String, Vec<u8>, bool, String)> = self
            .conn
            .prepare(&sql)?
            .query_map([value], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(id, name, keys, has_password, nick)| {
                Ok(StoredChannel {
                    id,
                    name,
                    keys: ChannelKeys::from_bytes(&keys)?,
                    has_password,
                    nick,
                })
            })
            .collect()
    }

    /// Joins a channel, or returns the id it already has.
    pub fn insert_channel(
        &self,
        name: &str,
        keys: &ChannelKeys,
        has_password: bool,
        nick: &str,
        now: u32,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO channels (name, channel_id, keys, has_password, nick, joined_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![name, keys.id(), keys.to_bytes(), has_password, nick, now],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM channels WHERE channel_id = ?",
            [keys.id()],
            |row| row.get(0),
        )?)
    }

    pub fn set_channel_nick(&self, id: i64, nick: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE channels SET nick = ? WHERE id = ?",
            params![nick, id],
        )?;
        Ok(())
    }

    /// Leaves a channel and forgets its posts.
    pub fn delete_channel(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM channel_muted WHERE channel_ref = ?", [id])?;
        self.conn
            .execute("DELETE FROM channel_posts WHERE channel_ref = ?", [id])?;
        self.conn
            .execute("DELETE FROM channels WHERE id = ?", [id])?;
        Ok(())
    }

    /// Records a post. Returns its id, or `None` if it is recorded already.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_post(
        &self,
        channel: i64,
        author: &[u8; 32],
        outgoing: bool,
        nick: &str,
        card: bool,
        bundle_id: BundleId,
        body: &str,
        created_at: u32,
        received_at: Option<u32>,
    ) -> Result<Option<i64>> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO channel_posts
             (channel_ref, author, outgoing, nick, card, bundle_id, body, created_at, received_at, read_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                channel,
                author,
                outgoing,
                nick,
                card,
                bundle_id.0,
                body,
                created_at,
                received_at,
                // Our own posts need no reading.
                outgoing.then_some(created_at),
            ],
        )?;
        Ok((inserted == 1).then(|| self.conn.last_insert_rowid()))
    }

    /// A channel's posts, without those of muted authors.
    pub fn posts(&self, channel: i64) -> Result<Vec<StoredPost>> {
        let mut statement = self.conn.prepare(
            "SELECT id, author, outgoing, nick, card, body, created_at, read_at IS NOT NULL
             FROM channel_posts p WHERE channel_ref = ?1 AND NOT EXISTS (
                 SELECT 1 FROM channel_muted m WHERE m.channel_ref = ?1 AND m.author = p.author
             )
             ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([channel], |row| {
            Ok(StoredPost {
                id: row.get(0)?,
                author: row.get(1)?,
                outgoing: row.get(2)?,
                nick: row.get(3)?,
                card: row.get(4)?,
                body: row.get(5)?,
                created_at: row.get(6)?,
                read: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn has_post(&self, id: BundleId) -> Result<bool> {
        self.exists("SELECT 1 FROM channel_posts WHERE bundle_id = ?", id)
    }

    pub fn last_post(&self, channel: i64) -> Result<Option<(String, u32)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT body, created_at FROM channel_posts WHERE channel_ref = ? AND card = 0
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                [channel],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub fn channel_unread_count(&self, channel: i64) -> Result<usize> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM channel_posts p WHERE channel_ref = ?1 AND read_at IS NULL
             AND NOT EXISTS (
                 SELECT 1 FROM channel_muted m WHERE m.channel_ref = ?1 AND m.author = p.author
             )",
            [channel],
            |row| row.get(0),
        )?)
    }

    pub fn mark_channel_read(&self, channel: i64, now: u32) -> Result<()> {
        self.conn.execute(
            "UPDATE channel_posts SET read_at = ? WHERE channel_ref = ? AND read_at IS NULL",
            params![now, channel],
        )?;
        Ok(())
    }

    /// Our posts in a channel since a time, for the rate limit.
    pub fn own_posts_since(&self, channel: i64, since: u32) -> Result<usize> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM channel_posts
             WHERE channel_ref = ? AND outgoing = 1 AND created_at >= ?",
            params![channel, since],
            |row| row.get(0),
        )?)
    }

    /// The authors of a channel's posts, for finding one by its short id.
    pub fn authors(&self, channel: i64) -> Result<Vec<[u8; 32]>> {
        let rows: Vec<[u8; 32]> = self
            .conn
            .prepare("SELECT DISTINCT author FROM channel_posts WHERE channel_ref = ?")?
            .query_map([channel], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn mute(&self, channel: i64, author: &[u8; 32]) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO channel_muted (channel_ref, author) VALUES (?, ?)",
            params![channel, author],
        )?;
        Ok(())
    }

    pub fn unmute(&self, channel: i64, author: &[u8; 32]) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM channel_muted WHERE channel_ref = ? AND author = ?",
            params![channel, author],
        )? == 1)
    }

    /// Muted authors of a channel, with the nickname of their latest post.
    pub fn muted(&self, channel: i64) -> Result<Vec<([u8; 32], Option<String>)>> {
        let rows: Vec<([u8; 32], Option<String>)> = self
            .conn
            .prepare(
                "SELECT m.author, (
                     SELECT nick FROM channel_posts p
                     WHERE p.channel_ref = m.channel_ref AND p.author = m.author
                     ORDER BY created_at DESC LIMIT 1
                 )
                 FROM channel_muted m WHERE m.channel_ref = ? ORDER BY m.rowid",
            )?
            .query_map([channel], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn is_muted(&self, channel: i64, author: &[u8; 32]) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM channel_muted WHERE channel_ref = ? AND author = ?",
                params![channel, author],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }
}

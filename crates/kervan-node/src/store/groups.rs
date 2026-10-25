//! Groups in the store (§5.7): states, sender chains and group messages.

use kervan_proto::bundle::BundleId;
use kervan_proto::group::{GroupId, GroupState, ReceiverChain, SenderChain, SkippedGroupKey};
use kervan_proto::identity::ContactCard;
use rusqlite::{params, OptionalExtension};

use super::{Store, MAX_SKIPPED_KEYS, SKIPPED_KEY_LIFETIME};
use crate::Result;

pub struct StoredGroup {
    pub id: i64,
    pub group_id: GroupId,
    pub name: String,
    /// The admin's X25519 key.
    pub admin: [u8; 32],
    pub version: u32,
    /// Whether we are still a member.
    pub member: bool,
    /// The members in the admin's order, the admin included.
    pub members: Vec<ContactCard>,
}

impl StoredGroup {
    pub fn has_member(&self, dh_key: &[u8; 32]) -> bool {
        self.members
            .iter()
            .any(|card| card.dh_key().as_bytes() == dh_key)
    }
}

pub struct StoredGroupMessage {
    pub id: i64,
    /// The sender's X25519 key, or `None` for our own.
    pub sender: Option<[u8; 32]>,
    pub body: String,
    pub created_at: u32,
    pub read: bool,
}

impl Store {
    pub fn groups(&self) -> Result<Vec<StoredGroup>> {
        let ids: Vec<i64> = self
            .conn
            .prepare("SELECT id FROM groups ORDER BY id")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut groups = Vec::new();
        for id in ids {
            groups.extend(self.group(id)?);
        }
        Ok(groups)
    }

    pub fn group(&self, id: i64) -> Result<Option<StoredGroup>> {
        self.group_where("id = ?", rusqlite::types::Value::Integer(id))
    }

    pub fn group_by_group_id(&self, group_id: &GroupId) -> Result<Option<StoredGroup>> {
        self.group_where("group_id = ?", group_id.0.to_vec().into())
    }

    fn group_where(
        &self,
        condition: &str,
        value: rusqlite::types::Value,
    ) -> Result<Option<StoredGroup>> {
        let row = self
            .conn
            .query_row(
                &format!(
                    "SELECT id, group_id, name, admin, version, member FROM groups WHERE {condition}"
                ),
                [value],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, [u8; 16]>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, [u8; 32]>(3)?,
                        row.get::<_, u32>(4)?,
                        row.get::<_, bool>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, group_id, name, admin, version, member)) = row else {
            return Ok(None);
        };
        let cards: Vec<Vec<u8>> = self
            .conn
            .prepare("SELECT card FROM group_members WHERE group_ref = ? ORDER BY position")?
            .query_map([id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let members = cards
            .iter()
            .map(|card| ContactCard::from_bytes(card))
            .collect::<kervan_proto::Result<_>>()?;
        Ok(Some(StoredGroup {
            id,
            group_id: GroupId(group_id),
            name,
            admin,
            version,
            member,
            members,
        }))
    }

    /// Saves a group state, replacing the members. Returns the group's id.
    pub fn save_group(
        &self,
        state: &GroupState,
        admin: &[u8; 32],
        member: bool,
        now: u32,
    ) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO groups (group_id, name, admin, version, member, created_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (group_id) DO UPDATE SET
                 name = excluded.name, version = excluded.version, member = excluded.member",
            params![state.id.0, state.name, admin, state.version, member, now],
        )?;
        let id: i64 = tx.query_row(
            "SELECT id FROM groups WHERE group_id = ?",
            [state.id.0],
            |row| row.get(0),
        )?;
        tx.execute("DELETE FROM group_members WHERE group_ref = ?", [id])?;
        for (position, card) in state.members.iter().enumerate() {
            tx.execute(
                "INSERT INTO group_members (group_ref, position, dh_key, card) VALUES (?, ?, ?, ?)",
                params![id, position, card.dh_key().as_bytes(), card.to_bytes()],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Our current chain for a group, with its row id.
    pub fn own_chain(&self, group_id: &GroupId) -> Result<Option<(i64, SenderChain)>> {
        let row: Option<(i64, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT id, state FROM sender_keys
                 WHERE group_id = ? AND own = 1 AND retired_at IS NULL",
                [group_id.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(id, state)| Ok((id, SenderChain::from_bytes(&state)?)))
            .transpose()
    }

    /// Starts using a new chain of our own for a group; the one before is
    /// deleted, since nobody needs it to read anything.
    pub fn insert_own_chain(
        &self,
        group_id: &GroupId,
        me: &[u8; 32],
        chain: &SenderChain,
        now: u32,
    ) -> Result<i64> {
        self.conn.execute(
            "DELETE FROM sender_keys WHERE group_id = ? AND own = 1",
            [group_id.0],
        )?;
        self.conn.execute(
            "INSERT INTO sender_keys (group_id, sender, chain_id, state, own, created_at)
             VALUES (?, ?, ?, ?, 1, ?)",
            params![group_id.0, me, chain.id(), chain.to_bytes(), now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn save_chain(&self, id: i64, state: &[u8]) -> Result<()> {
        self.conn.execute(
            "UPDATE sender_keys SET state = ? WHERE id = ?",
            params![state, id],
        )?;
        Ok(())
    }

    /// Keeps another member's chain. A chain from the same member that was
    /// sent earlier is retired, and one that was sent later retires this one.
    /// Returns the new chain's row id, or `None` if it was known.
    pub fn insert_receiver_chain(
        &self,
        group_id: &GroupId,
        sender: &[u8; 32],
        chain: &ReceiverChain,
        sent_at: u32,
        now: u32,
    ) -> Result<Option<i64>> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO sender_keys (group_id, sender, chain_id, state, own, created_at)
             VALUES (?, ?, ?, ?, 0, ?)",
            params![group_id.0, sender, chain.id(), chain.to_bytes(), sent_at],
        )?;
        if inserted == 0 {
            return Ok(None);
        }
        let id = self.conn.last_insert_rowid();
        self.conn.execute(
            "UPDATE sender_keys SET retired_at = ?1
             WHERE group_id = ?2 AND sender = ?3 AND own = 0 AND retired_at IS NULL AND id != ?4
               AND created_at <= ?5",
            params![now, group_id.0, sender, id, sent_at],
        )?;
        self.conn.execute(
            "UPDATE sender_keys SET retired_at = ?1
             WHERE id = ?4 AND EXISTS (
                 SELECT 1 FROM sender_keys WHERE group_id = ?2 AND sender = ?3 AND own = 0
                     AND id != ?4 AND created_at > ?5
             )",
            params![now, group_id.0, sender, id, sent_at],
        )?;
        Ok(Some(id))
    }

    /// Another member's chain by row id, with its group and sender.
    pub fn receiver_chain(&self, id: i64) -> Result<Option<(GroupId, [u8; 32], ReceiverChain)>> {
        let row: Option<([u8; 16], [u8; 32], Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT group_id, sender, state FROM sender_keys WHERE id = ? AND own = 0",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        row.map(|(group_id, sender, state)| {
            Ok((
                GroupId(group_id),
                sender,
                ReceiverChain::from_bytes(&state)?,
            ))
        })
        .transpose()
    }

    /// Every chain of another member, for the group tags.
    pub fn receiver_chains(&self) -> Result<Vec<(i64, ReceiverChain)>> {
        let rows: Vec<(i64, Vec<u8>)> = self
            .conn
            .prepare("SELECT id, state FROM sender_keys WHERE own = 0")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(id, state)| Ok((id, ReceiverChain::from_bytes(&state)?)))
            .collect()
    }

    pub fn group_skipped_key(&self, chain: i64, iteration: u32) -> Result<Option<[u8; 32]>> {
        Ok(self
            .conn
            .query_row(
                "SELECT key FROM group_skipped WHERE chain = ? AND iteration = ?",
                params![chain, iteration],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn delete_group_skipped_key(&self, chain: i64, iteration: u32) -> Result<()> {
        self.conn.execute(
            "DELETE FROM group_skipped WHERE chain = ? AND iteration = ?",
            params![chain, iteration],
        )?;
        Ok(())
    }

    pub fn store_group_skipped_keys(
        &self,
        chain: i64,
        keys: &[SkippedGroupKey],
        now: u32,
    ) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        for (iteration, key) in keys {
            tx.execute(
                "INSERT OR REPLACE INTO group_skipped (chain, iteration, key, stored_at)
                 VALUES (?, ?, ?, ?)",
                params![chain, iteration, key, now],
            )?;
        }
        tx.execute(
            "DELETE FROM group_skipped WHERE chain = ?1 AND rowid NOT IN (
                 SELECT rowid FROM group_skipped WHERE chain = ?1
                 ORDER BY stored_at DESC, iteration DESC LIMIT ?2
             )",
            params![chain, MAX_SKIPPED_KEYS],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Forgets every chain of a group, ours included: we left it.
    pub fn delete_group_chains(&self, group_id: &GroupId) -> Result<()> {
        self.conn.execute(
            "DELETE FROM group_skipped WHERE chain IN (SELECT id FROM sender_keys WHERE group_id = ?)",
            [group_id.0],
        )?;
        self.conn
            .execute("DELETE FROM sender_keys WHERE group_id = ?", [group_id.0])?;
        Ok(())
    }

    /// Records a group message. Returns its id, or `None` if it is recorded.
    pub fn insert_group_message(
        &self,
        group: i64,
        sender: Option<&[u8; 32]>,
        bundle_id: BundleId,
        body: &str,
        created_at: u32,
        received_at: Option<u32>,
    ) -> Result<Option<i64>> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO group_messages
             (group_ref, sender, bundle_id, body, created_at, received_at, read_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                group,
                sender,
                bundle_id.0,
                body,
                created_at,
                received_at,
                // Our own messages need no reading.
                sender.is_none().then_some(created_at),
            ],
        )?;
        Ok((inserted == 1).then(|| self.conn.last_insert_rowid()))
    }

    pub fn group_messages(&self, group: i64) -> Result<Vec<StoredGroupMessage>> {
        let mut statement = self.conn.prepare(
            "SELECT id, sender, body, created_at, read_at IS NOT NULL
             FROM group_messages WHERE group_ref = ? ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([group], |row| {
            Ok(StoredGroupMessage {
                id: row.get(0)?,
                sender: row.get(1)?,
                body: row.get(2)?,
                created_at: row.get(3)?,
                read: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn has_group_message(&self, id: BundleId) -> Result<bool> {
        self.exists("SELECT 1 FROM group_messages WHERE bundle_id = ?", id)
    }

    pub fn last_group_message(&self, group: i64) -> Result<Option<(String, u32)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT body, created_at FROM group_messages WHERE group_ref = ?
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                [group],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub fn group_unread_count(&self, group: i64) -> Result<usize> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM group_messages WHERE group_ref = ? AND read_at IS NULL",
            [group],
            |row| row.get(0),
        )?)
    }

    pub fn mark_group_read(&self, group: i64, now: u32) -> Result<()> {
        self.conn.execute(
            "UPDATE group_messages SET read_at = ? WHERE group_ref = ? AND read_at IS NULL",
            params![now, group],
        )?;
        Ok(())
    }

    /// Chains retired over 8 days ago, and chains of groups we never heard
    /// of after 8 days, go with their skipped keys.
    pub(super) fn prune_groups(&self, now: u32) -> Result<()> {
        let before = now.saturating_sub(SKIPPED_KEY_LIFETIME);
        let stale = "SELECT id FROM sender_keys WHERE retired_at <= ?1
             OR (created_at <= ?1 AND group_id NOT IN (SELECT group_id FROM groups))";
        self.conn.execute(
            &format!("DELETE FROM group_skipped WHERE chain IN ({stale}) OR stored_at <= ?1"),
            [before],
        )?;
        self.conn.execute(
            &format!("DELETE FROM sender_keys WHERE id IN ({stale})"),
            [before],
        )?;
        Ok(())
    }
}

//! Files in the store (§5.9): offers, chunks still arriving, and whole files.

use kervan_proto::file::FileOffer;
use rusqlite::{params, OptionalExtension};

use super::Store;
use crate::Result;

pub struct StoredFile {
    pub id: i64,
    pub offer: FileOffer,
    /// Offered in a conversation between two contacts: chunks we take are
    /// tombstoned rather than carried on.
    pub pair: bool,
    /// Whether all of it is here.
    pub ready: bool,
    pub received: u32,
    pub expires_at: u32,
}

impl Store {
    /// Records an offer, or returns the id it already has. `data` is the
    /// whole file when it is our own.
    pub fn insert_file(
        &self,
        offer: &FileOffer,
        pair: bool,
        data: Option<&[u8]>,
        expires_at: u32,
    ) -> Result<i64> {
        let bytes = offer.to_bytes();
        self.conn.execute(
            "INSERT OR IGNORE INTO files (offer, pair, data, expires_at) VALUES (?, ?, ?, ?)",
            params![bytes, pair, data, expires_at],
        )?;
        Ok(self
            .conn
            .query_row("SELECT id FROM files WHERE offer = ?", [bytes], |row| {
                row.get(0)
            })?)
    }

    pub fn file(&self, id: i64) -> Result<Option<StoredFile>> {
        let row: Option<(Vec<u8>, bool, bool, u32, u32)> = self
            .conn
            .query_row(
                "SELECT offer, pair, data IS NOT NULL, expires_at,
                     (SELECT COUNT(*) FROM file_chunks WHERE file_ref = files.id)
                 FROM files WHERE id = ?",
                [id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((offer, pair, ready, expires_at, received)) = row else {
            return Ok(None);
        };
        let offer = FileOffer::from_bytes(&offer)?;
        let received = if ready { offer.chunks() } else { received };
        Ok(Some(StoredFile {
            id,
            offer,
            pair,
            ready,
            received,
            expires_at,
        }))
    }

    /// Files still arriving whose chunks may still come.
    pub fn incomplete_files(&self, now: u32) -> Result<Vec<(i64, FileOffer)>> {
        let rows: Vec<(i64, Vec<u8>)> = self
            .conn
            .prepare("SELECT id, offer FROM files WHERE data IS NULL AND expires_at > ?")?
            .query_map([now], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(id, offer)| Ok((id, FileOffer::from_bytes(&offer)?)))
            .collect()
    }

    /// Keeps a chunk. Returns whether it was new.
    pub fn insert_file_chunk(&self, file: i64, index: u32, data: &[u8]) -> Result<bool> {
        Ok(self.conn.execute(
            "INSERT OR IGNORE INTO file_chunks (file_ref, idx, data) VALUES (?, ?, ?)",
            params![file, index, data],
        )? == 1)
    }

    pub fn file_chunks(&self, file: i64) -> Result<Vec<Vec<u8>>> {
        let rows: Vec<Vec<u8>> = self
            .conn
            .prepare("SELECT data FROM file_chunks WHERE file_ref = ? ORDER BY idx")?
            .query_map([file], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Keeps the whole file and lets go of its chunks.
    pub fn complete_file(&self, file: i64, data: &[u8]) -> Result<()> {
        self.conn.execute(
            "UPDATE files SET data = ? WHERE id = ?",
            params![data, file],
        )?;
        self.conn
            .execute("DELETE FROM file_chunks WHERE file_ref = ?", [file])?;
        Ok(())
    }

    pub fn file_data(&self, file: i64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row("SELECT data FROM files WHERE id = ?", [file], |row| {
                row.get(0)
            })
            .optional()?
            .flatten())
    }

    /// Ties a message, group message or channel post to the file it offers.
    pub fn attach_file(&self, table: FileTable, row: i64, file: i64) -> Result<()> {
        let table = match table {
            FileTable::Messages => "messages",
            FileTable::GroupMessages => "group_messages",
            FileTable::Posts => "channel_posts",
        };
        self.conn.execute(
            &format!("UPDATE {table} SET file_ref = ? WHERE id = ?"),
            params![file, row],
        )?;
        Ok(())
    }

    /// The file a message, group message or channel post offers.
    pub fn attached_file(&self, table: FileTable, row: i64) -> Result<Option<i64>> {
        let table = match table {
            FileTable::Messages => "messages",
            FileTable::GroupMessages => "group_messages",
            FileTable::Posts => "channel_posts",
        };
        Ok(self
            .conn
            .query_row(
                &format!("SELECT file_ref FROM {table} WHERE id = ?"),
                [row],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Chunks of files that can no longer arrive go.
    pub(super) fn prune_files(&self, now: u32) -> Result<()> {
        self.conn.execute(
            "DELETE FROM file_chunks WHERE file_ref IN (
                 SELECT id FROM files WHERE data IS NULL AND expires_at <= ?
             )",
            [now],
        )?;
        Ok(())
    }
}

/// Where a file offer was recorded.
#[derive(Clone, Copy, Debug)]
pub enum FileTable {
    Messages,
    GroupMessages,
    Posts,
}

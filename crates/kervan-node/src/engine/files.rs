//! Files (§5.9): offering photos and voice messages, and taking their chunks.

use kervan_proto::bundle::{Bundle, Message, Tombstone, DEFAULT_HOP_LIMIT, DEFAULT_TTL_SECS};
use kervan_proto::channel::{CHANNEL_HOP_LIMIT, CHANNEL_TTL_SECS};
use kervan_proto::file::{assemble, seal_file, FileOffer};
use kervan_proto::frame::Frame;
use kervan_proto::pair::RecipientTag;
use rand_core::OsRng;

use super::{forwarded, Engine, Handled, Outcome, MAX_FUTURE_EXPIRY_SECS};
use crate::store::{FileTable, StoredFile};
use crate::{Error, Event, FileView, GroupMessageView, MessageView, PostView, Result};

/// Own frames to send: the offer, then the chunks, each with its tag.
pub type Outgoing = Vec<(Frame, RecipientTag)>;

impl Engine {
    /// Sends a file to a contact: the offer in their conversation, and the
    /// chunks, which they tombstone as they take them.
    pub fn send_file(
        &mut self,
        contact_id: i64,
        mime: &str,
        data: &[u8],
        now: u32,
    ) -> Result<(MessageView, Outgoing)> {
        let index = self.contact_index(contact_id)?;
        let contact = &self.contacts[index];
        if contact.request || contact.hidden {
            return Err(Error::Invalid("accept the request before you reply"));
        }
        if !contact.introduced {
            return Err(Error::Invalid(
                "write to them once first: they need your card to get a file",
            ));
        }
        let expires_at = now + DEFAULT_TTL_SECS;
        let (offer, file, chunks) =
            self.offer_file(mime, data, expires_at, DEFAULT_HOP_LIMIT, true, now)?;
        let bundle = self.seal_pair(index, &Message::file(&offer, now), now)?;
        self.store.insert_bundle(&bundle, true, now)?;
        let mut message = self
            .store
            .insert_message(contact_id, true, bundle.id, "", now, None)?
            .ok_or(Error::Invalid("message already exists"))?;
        self.store
            .attach_file(FileTable::Messages, message.id, file)?;
        message.file = self.file_view(file)?;
        let mut frames: Outgoing = forwarded(&bundle)
            .map(|frame| (frame, bundle.tag))
            .into_iter()
            .collect();
        frames.extend(chunks);
        Ok((message, frames))
    }

    /// Sends a file to a group: one set of chunks for every member.
    pub fn send_group_file(
        &mut self,
        group_id: i64,
        mime: &str,
        data: &[u8],
        now: u32,
    ) -> Result<(GroupMessageView, Outgoing)> {
        let expires_at = now + DEFAULT_TTL_SECS;
        let (offer, file, chunks) =
            self.offer_file(mime, data, expires_at, DEFAULT_HOP_LIMIT, false, now)?;
        let (mut message, frame) =
            self.send_group_message(group_id, &Message::file(&offer, now), "", now)?;
        self.store
            .attach_file(FileTable::GroupMessages, message.id, file)?;
        message.file = self.file_view(file)?;
        let mut frames = tagged(frame);
        frames.extend(chunks);
        Ok((message, frames))
    }

    /// Posts a file to a channel: anyone in it can take the chunks.
    pub fn send_channel_file(
        &mut self,
        channel_id: i64,
        mime: &str,
        data: &[u8],
        now: u32,
    ) -> Result<(PostView, Outgoing)> {
        let expires_at = now + CHANNEL_TTL_SECS;
        let (offer, file, chunks) =
            self.offer_file(mime, data, expires_at, CHANNEL_HOP_LIMIT, false, now)?;
        let (mut post, frame) = self.post(channel_id, Message::file(&offer, now), now)?;
        self.store.attach_file(FileTable::Posts, post.id, file)?;
        post.file = self.file_view(file)?;
        let mut frames = tagged(frame);
        frames.extend(chunks);
        Ok((post, frames))
    }

    /// A whole file and its media type, once it is here.
    pub fn file_data(&self, file_id: i64) -> Result<Option<(String, Vec<u8>)>> {
        let Some(file) = self.store.file(file_id)? else {
            return Ok(None);
        };
        Ok(self
            .store
            .file_data(file_id)?
            .map(|data| (file.offer.mime.clone(), data)))
    }

    /// Records an offer from a message we opened, and takes the chunks we
    /// already carry for it. Returns the file, or `None` if the offer is bad.
    pub(super) fn take_offer(
        &mut self,
        body: &[u8],
        pair: bool,
        expires_at: u32,
        now: u32,
    ) -> Result<Option<(i64, Vec<Handled>)>> {
        let Ok(offer) = FileOffer::from_bytes(body) else {
            return Ok(None);
        };
        let file = self.store.insert_file(&offer, pair, None, expires_at)?;
        self.forget_tags();
        let opened = self.open_carried_chunks(now)?;
        Ok(Some((file, opened)))
    }

    /// Takes a chunk frame: ours if it belongs to a file we were offered,
    /// otherwise carried like anybody else's.
    pub(super) fn handle_chunk(
        &mut self,
        bundle: Bundle,
        now: u32,
        carry: bool,
    ) -> Result<Handled> {
        if bundle.is_expired(now) || bundle.expires_at > now.saturating_add(MAX_FUTURE_EXPIRY_SECS)
        {
            return Ok(Handled::quiet(Outcome::Dropped));
        }
        if self.store.has_bundle(bundle.id)? || self.store.has_tombstone(bundle.id)? {
            return Ok(Handled::quiet(Outcome::Duplicate));
        }
        self.refresh_tags(now);
        if let Some(&(file, index)) = self.file_tags.get(&bundle.tag) {
            if let Some(handled) = self.take_chunk(file, index, &bundle, now, false)? {
                return Ok(handled);
            }
        }
        if !carry {
            return Ok(Handled::quiet(Outcome::Dropped));
        }
        self.store.insert_chunk_bundle(&bundle, false, now)?;
        Ok(Handled {
            outcome: Outcome::Carried,
            relay: forwarded_chunk(&bundle).into_iter().collect(),
            events: Vec::new(),
        })
    }

    /// Takes chunk `index` of a file. In a pair conversation the chunk is
    /// tombstoned; in a group or channel it is carried on for the others.
    fn take_chunk(
        &mut self,
        file: i64,
        index: u32,
        bundle: &Bundle,
        now: u32,
        carried: bool,
    ) -> Result<Option<Handled>> {
        let Some(stored) = self.store.file(file)? else {
            return Ok(None);
        };
        if stored.ready {
            return Ok(None);
        }
        let keys = stored.offer.keys();
        let Ok(data) = keys.open_chunk(bundle) else {
            return Ok(None);
        };
        self.store.insert_file_chunk(file, index, &data)?;

        let mut relay = Vec::new();
        if stored.pair {
            let tombstone: Tombstone = keys.tombstone(bundle);
            self.store.insert_tombstone(&tombstone, bundle.expires_at)?;
            if carried {
                self.store.delete_bundle(bundle.id)?;
            }
            relay.push(Frame::Tombstone {
                hop_limit: DEFAULT_HOP_LIMIT,
                tombstone,
            });
        } else if !carried {
            self.store.insert_chunk_bundle(bundle, false, now)?;
            relay.extend(forwarded_chunk(bundle));
        }

        let chunks = self.store.file_chunks(file)?;
        if chunks.len() == stored.offer.chunks() as usize {
            // A file whose chunks do not add up keeps them, unread, until it expires.
            if let Ok(whole) = assemble(&stored.offer, &chunks) {
                self.store.complete_file(file, &whole)?;
                self.forget_tags();
            }
        }
        let events = self
            .file_view(file)?
            .map(|file| Event::FileChanged { file })
            .into_iter()
            .collect();
        Ok(Some(Handled {
            outcome: Outcome::Received,
            relay,
            events,
        }))
    }

    /// Takes the carried chunks of files we were just offered.
    fn open_carried_chunks(&mut self, now: u32) -> Result<Vec<Handled>> {
        self.refresh_tags(now);
        let mut opened = Vec::new();
        for bundle in self.store.carried_chunks(now)? {
            if let Some(&(file, index)) = self.file_tags.get(&bundle.tag) {
                opened.extend(self.take_chunk(file, index, &bundle, now, true)?);
            }
        }
        Ok(opened)
    }

    /// Seals a file and keeps its chunks as our own bundles until delivered
    /// or expired. Returns the offer, the file's id and the chunk frames.
    fn offer_file(
        &mut self,
        mime: &str,
        data: &[u8],
        expires_at: u32,
        hop_limit: u8,
        pair: bool,
        now: u32,
    ) -> Result<(FileOffer, i64, Outgoing)> {
        let (offer, chunks) = seal_file(mime, data, expires_at, hop_limit, &mut OsRng)?;
        let file = self
            .store
            .insert_file(&offer, pair, Some(data), expires_at)?;
        let mut frames = Vec::with_capacity(chunks.len());
        for chunk in &chunks {
            self.store.insert_chunk_bundle(chunk, true, now)?;
            frames.extend(forwarded_chunk(chunk).map(|frame| (frame, chunk.tag)));
        }
        Ok((offer, file, frames))
    }

    pub(super) fn file_view(&self, file_id: i64) -> Result<Option<FileView>> {
        Ok(self.store.file(file_id)?.as_ref().map(file_view))
    }

    /// The file a message, group message or post offers, if any.
    pub(super) fn attached(&self, table: FileTable, row: i64) -> Result<Option<FileView>> {
        match self.store.attached_file(table, row)? {
            Some(file) => self.file_view(file),
            None => Ok(None),
        }
    }
}

fn file_view(file: &StoredFile) -> FileView {
    FileView {
        id: file.id,
        mime: file.offer.mime.clone(),
        size: file.offer.size,
        chunks: file.offer.chunks(),
        received: file.received,
        ready: file.ready,
        expires_at: file.expires_at,
    }
}

fn tagged(frame: Frame) -> Outgoing {
    match &frame {
        Frame::Bundle(bundle) => {
            let tag = bundle.tag;
            vec![(frame, tag)]
        }
        _ => Vec::new(),
    }
}

/// A chunk as this node passes it on, with one hop used (§8).
pub(super) fn forwarded_chunk(bundle: &Bundle) -> Option<Frame> {
    match forwarded(bundle)? {
        Frame::Bundle(bundle) => Some(Frame::Chunk(bundle)),
        other => Some(other),
    }
}

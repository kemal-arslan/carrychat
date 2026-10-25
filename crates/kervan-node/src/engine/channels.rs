//! Public channels (§5.8): joining, posting and reading.

use data_encoding::BASE32_NOPAD;
use kervan_proto::bundle::{Bundle, Kind, Message};
use kervan_proto::channel::{
    author_key, normalize_name, ChannelKeys, CHANNEL_HOP_LIMIT, CHANNEL_TTL_SECS, MAX_NICK_LEN,
};
use kervan_proto::frame::Frame;
use kervan_proto::identity::ContactCard;
use rand_core::OsRng;

use super::{forwarded, Engine, Handled, Outcome};
use crate::store::{FileTable, StoredChannel, StoredPost};
use crate::{ChannelView, Error, Event, FileView, MutedView, PostView, Result, SharedCardView};

/// Posts we send to one channel in a minute, at most.
const MAX_POSTS_PER_MINUTE: usize = 10;

impl Engine {
    pub fn channels(&self) -> Result<Vec<ChannelView>> {
        self.store
            .channels()?
            .iter()
            .map(|channel| self.channel_view(channel))
            .collect()
    }

    /// Joins a channel, posting under `nick`. Posts this node already
    /// carries for it are opened right away; their results are returned.
    pub fn join_channel(
        &mut self,
        name: &str,
        password: &str,
        nick: &str,
        now: u32,
    ) -> Result<(ChannelView, Vec<Handled>)> {
        let name = normalize_name(name)?;
        let nick = check_nick(nick)?;
        let keys = ChannelKeys::derive(&name, password)?;
        let id = self
            .store
            .insert_channel(&name, &keys, !password.is_empty(), &nick, now)?;
        self.forget_tags();
        let opened = self.open_carried_posts(now)?;
        Ok((self.channel_view(&self.stored_channel(id)?)?, opened))
    }

    /// Leaves a channel and forgets its posts.
    pub fn leave_channel(&mut self, channel_id: i64) -> Result<()> {
        self.stored_channel(channel_id)?;
        self.store.delete_channel(channel_id)?;
        self.forget_tags();
        Ok(())
    }

    pub fn set_channel_nick(&mut self, channel_id: i64, nick: &str) -> Result<ChannelView> {
        self.stored_channel(channel_id)?;
        self.store
            .set_channel_nick(channel_id, &check_nick(nick)?)?;
        self.channel_view(&self.stored_channel(channel_id)?)
    }

    pub fn posts(&self, channel_id: i64) -> Result<Vec<PostView>> {
        let channel = self.stored_channel(channel_id)?;
        self.store
            .posts(channel.id)?
            .into_iter()
            .map(|post| {
                let file = self.attached(FileTable::Posts, post.id)?;
                Ok(post_view(channel.id, post, file))
            })
            .collect()
    }

    pub fn send_post(
        &mut self,
        channel_id: i64,
        text: &str,
        now: u32,
    ) -> Result<(PostView, Frame)> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::Invalid("message is empty"));
        }
        self.post(channel_id, Message::text(text, now), now)
    }

    /// Posts our contact card, so readers can add us. It links our posts in
    /// this channel to our card, and nothing else.
    pub fn share_card(&mut self, channel_id: i64, now: u32) -> Result<(PostView, Frame)> {
        self.post(channel_id, Message::card(&self.identity.card(), now), now)
    }

    /// Mutes an author, named by the short id posts show: their posts are
    /// neither shown nor passed on.
    pub fn mute_author(&mut self, channel_id: i64, author: &str) -> Result<()> {
        let channel = self.stored_channel(channel_id)?;
        let own = author_key(&self.author_seed(&channel));
        let key = self
            .store
            .authors(channel.id)?
            .into_iter()
            .find(|key| short_id(key) == author && key != &own)
            .ok_or(Error::Invalid("no such author in this channel"))?;
        self.store.mute(channel.id, &key)
    }

    /// Shows an author's posts again. Their posts that were dropped while
    /// muted are taken the next time a link offers them.
    pub fn unmute_author(&mut self, channel_id: i64, author: &str) -> Result<()> {
        let channel = self.stored_channel(channel_id)?;
        let key = self
            .store
            .muted(channel.id)?
            .into_iter()
            .map(|(key, _)| key)
            .find(|key| short_id(key) == author)
            .ok_or(Error::Invalid("that author is not muted here"))?;
        self.store.unmute(channel.id, &key)?;
        Ok(())
    }

    pub fn muted_authors(&self, channel_id: i64) -> Result<Vec<MutedView>> {
        let channel = self.stored_channel(channel_id)?;
        Ok(self
            .store
            .muted(channel.id)?
            .into_iter()
            .map(|(key, nick)| MutedView {
                author: short_id(&key),
                nick: nick.unwrap_or_default(),
            })
            .collect())
    }

    pub fn mark_channel_read(&mut self, channel_id: i64, now: u32) -> Result<()> {
        self.stored_channel(channel_id)?;
        self.store.mark_channel_read(channel_id, now)
    }

    /// Opens a post in one of our channels. A post we already carry is not
    /// stored or passed on again.
    pub(super) fn receive_post(
        &mut self,
        bundle: &Bundle,
        now: u32,
        carried: bool,
    ) -> Result<Option<Handled>> {
        self.refresh_tags(now);
        let Some(&channel_id) = self.channel_tags.get(&bundle.tag) else {
            return Ok(None);
        };
        let Some(channel) = self.store.channel(channel_id)? else {
            return Ok(None);
        };
        let Ok(post) = channel.keys.open(bundle) else {
            return Ok(None);
        };
        if self.store.is_muted(channel.id, &post.author)? {
            return Ok(Some(Handled::quiet(Outcome::Dropped)));
        }

        let mut events = Vec::new();
        let card = post.message.kind == Kind::Card;
        let mut file = None;
        let body = match post.message.kind {
            Kind::Text => Some(String::from_utf8_lossy(&post.message.body).into_owned()),
            Kind::Card => ContactCard::from_bytes(&post.message.body)
                .ok()
                .map(|card| card.to_text()),
            Kind::File => {
                match self.take_offer(&post.message.body, false, bundle.expires_at, now)? {
                    Some((taken, opened)) => {
                        file = Some(taken);
                        events.extend(opened.into_iter().flat_map(|handled| handled.events));
                        Some(String::new())
                    }
                    None => None,
                }
            }
            _ => None,
        };
        if let Some(body) = body {
            let stored = StoredPost {
                id: 0,
                author: post.author,
                outgoing: false,
                nick: post.nick,
                card,
                body,
                created_at: post.message.created_at,
                read: false,
            };
            if let Some(id) = self.store.insert_post(
                channel.id,
                &stored.author,
                false,
                &stored.nick,
                card,
                bundle.id,
                &stored.body,
                stored.created_at,
                Some(now),
            )? {
                let view = match file {
                    Some(file) => {
                        self.store.attach_file(FileTable::Posts, id, file)?;
                        self.file_view(file)?
                    }
                    None => None,
                };
                events.push(Event::PostReceived {
                    post: post_view(channel.id, StoredPost { id, ..stored }, view),
                });
            }
        }
        // Members carry posts on for one another, within the channel's hops.
        let relay = if carried {
            Vec::new()
        } else {
            let capped = Bundle {
                hop_limit: bundle.hop_limit.min(CHANNEL_HOP_LIMIT),
                ..bundle.clone()
            };
            self.store.insert_bundle(&capped, false, now)?;
            forwarded(&capped).into_iter().collect()
        };
        Ok(Some(Handled {
            outcome: Outcome::Received,
            relay,
            events,
        }))
    }

    /// Opens the carried posts of channels we just joined.
    fn open_carried_posts(&mut self, now: u32) -> Result<Vec<Handled>> {
        self.refresh_tags(now);
        let mut opened = Vec::new();
        for bundle in self.store.carried_bundles(now)? {
            if self.channel_tags.contains_key(&bundle.tag) && !self.store.has_post(bundle.id)? {
                opened.extend(self.receive_post(&bundle, now, true)?);
            }
        }
        Ok(opened)
    }

    pub(super) fn post(
        &mut self,
        channel_id: i64,
        message: Message,
        now: u32,
    ) -> Result<(PostView, Frame)> {
        let channel = self.stored_channel(channel_id)?;
        if self
            .store
            .own_posts_since(channel.id, now.saturating_sub(60))?
            >= MAX_POSTS_PER_MINUTE
        {
            return Err(Error::Invalid("at most 10 posts a minute in a channel"));
        }
        let seed = self.author_seed(&channel);
        let bundle = channel.keys.seal(
            &seed,
            &channel.nick,
            &message,
            now + CHANNEL_TTL_SECS,
            CHANNEL_HOP_LIMIT,
            &mut OsRng,
        )?;
        self.store.insert_bundle(&bundle, true, now)?;
        let card = message.kind == Kind::Card;
        let body = match message.kind {
            Kind::Card => self.identity.card().to_text(),
            Kind::File => String::new(),
            _ => String::from_utf8_lossy(&message.body).into_owned(),
        };
        let author = author_key(&seed);
        let id = self
            .store
            .insert_post(
                channel.id,
                &author,
                true,
                &channel.nick,
                card,
                bundle.id,
                &body,
                now,
                None,
            )?
            .ok_or(Error::Invalid("message already exists"))?;
        let post = StoredPost {
            id,
            author,
            outgoing: true,
            nick: channel.nick.clone(),
            card,
            body,
            created_at: now,
            read: true,
        };
        let frame = forwarded(&bundle).expect("a new bundle has hops left");
        Ok((post_view(channel.id, post, None), frame))
    }

    fn author_seed(&self, channel: &StoredChannel) -> [u8; 32] {
        channel.keys.author_seed(&self.identity.secret_bytes().0)
    }

    fn stored_channel(&self, channel_id: i64) -> Result<StoredChannel> {
        self.store
            .channel(channel_id)?
            .ok_or(Error::Invalid("unknown channel"))
    }

    fn channel_view(&self, channel: &StoredChannel) -> Result<ChannelView> {
        let last = self.store.last_post(channel.id)?;
        Ok(ChannelView {
            id: channel.id,
            name: channel.name.clone(),
            has_password: channel.has_password,
            nick: channel.nick.clone(),
            last_message: last.as_ref().map(|(body, _)| body.clone()),
            last_at: last.map(|(_, at)| at),
            unread: self.store.channel_unread_count(channel.id)?,
        })
    }
}

fn post_view(channel_id: i64, post: StoredPost, file: Option<FileView>) -> PostView {
    let shared_card = post
        .card
        .then(|| ContactCard::from_text(&post.body).ok())
        .flatten()
        .map(|card| SharedCardView {
            name: card.name().to_owned(),
            fingerprint: card.fingerprint().to_string(),
            card: card.to_text(),
        });
    PostView {
        id: post.id,
        channel_id,
        outgoing: post.outgoing,
        nick: post.nick,
        author: short_id(&post.author),
        body: if post.card { String::new() } else { post.body },
        shared_card,
        created_at: post.created_at,
        read: post.read,
        file,
    }
}

/// Eight base32 characters of an author's key: enough to tell apart two
/// authors with the same nickname.
fn short_id(key: &[u8; 32]) -> String {
    BASE32_NOPAD.encode(&key[..5]).to_lowercase()
}

fn check_nick(nick: &str) -> Result<String> {
    let nick = nick.trim();
    if nick.is_empty() {
        return Err(Error::Invalid("choose a nickname"));
    }
    if nick.len() > MAX_NICK_LEN {
        return Err(Error::Invalid("that nickname is too long"));
    }
    Ok(nick.to_owned())
}

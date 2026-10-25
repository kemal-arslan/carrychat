import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  type Channel,
  type Contact,
  type FileView,
  type Group,
  type GroupMessage,
  type Member,
  type Message,
  type Post,
  type Profile,
  type Status,
} from "./api";
import { ChannelChat } from "./ChannelChat";
import { Chat } from "./Chat";
import {
  AddContactDialog,
  ChannelSettingsDialog,
  ConnectDialog,
  JoinChannelDialog,
  MembersDialog,
  MyCardDialog,
  NetworkDialog,
  NewGroupDialog,
  SettingsDialog,
} from "./Dialogs";
import { GroupChat } from "./GroupChat";
import { shortTime } from "./format";
import { useI18n } from "./i18n";
import { Logo } from "./Logo";
import { notificationsEnabled } from "./prefs";
import { Setup } from "./Setup";
import { StatusBar } from "./StatusBar";

export default function App() {
  const [profile, setProfile] = useState<Profile | null | undefined>(undefined);

  useEffect(() => {
    api.profile().then(setProfile);
  }, []);

  if (profile === undefined) return null;
  if (profile === null) return <Setup onCreated={setProfile} />;
  return <Main profile={profile} onProfile={setProfile} />;
}

type Notice = { text: string; action?: { label: string; run: () => void } };

type DialogName =
  | "add"
  | "card"
  | "connect"
  | "network"
  | "settings"
  | "newGroup"
  | "members"
  | "joinChannel"
  | "channelSettings";

function Main({ profile, onProfile }: { profile: Profile; onProfile: (profile: Profile) => void }) {
  const { t, language } = useI18n();
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [groups, setGroups] = useState<Group[]>([]);
  const [selectedGroup, setSelectedGroup] = useState<number | null>(null);
  const [groupMessages, setGroupMessages] = useState<GroupMessage[]>([]);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [selectedChannel, setSelectedChannel] = useState<number | null>(null);
  const [posts, setPosts] = useState<Post[]>([]);
  const [status, setStatus] = useState<Status | null>(null);
  const [dialog, setDialog] = useState<DialogName | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const contactsRef = useRef(contacts);
  contactsRef.current = contacts;
  const selectedGroupRef = useRef(selectedGroup);
  selectedGroupRef.current = selectedGroup;
  const groupsRef = useRef(groups);
  groupsRef.current = groups;
  const selectedChannelRef = useRef(selectedChannel);
  selectedChannelRef.current = selectedChannel;
  const channelsRef = useRef(channels);
  channelsRef.current = channels;
  const tRef = useRef(t);
  tRef.current = t;

  const refreshContacts = useCallback(() => api.contacts().then(setContacts), []);
  const refreshGroups = useCallback(() => api.groups().then(setGroups), []);
  const refreshChannels = useCallback(() => api.channels().then(setChannels), []);
  const refreshStatus = useCallback(() => api.status().then(setStatus), []);

  const show = useCallback((text: string, action?: Notice["action"]) => {
    const next = { text, action };
    setNotice(next);
    // A notice with an action stays a little longer, to give time to use it.
    window.setTimeout(() => setNotice((current) => (current === next ? null : current)), action ? 8000 : 5000);
  }, []);

  // Messages count as read only while someone is looking at them.
  const markSelectedRead = useCallback(() => {
    if (!document.hasFocus()) return;
    const contactId = selectedRef.current;
    const groupId = selectedGroupRef.current;
    if (contactId !== null) api.markRead(contactId).then(refreshContacts);
    if (groupId !== null) api.markGroupRead(groupId).then(refreshGroups);
    const channelId = selectedChannelRef.current;
    if (channelId !== null) api.markChannelRead(channelId).then(refreshChannels);
  }, [refreshContacts, refreshGroups, refreshChannels]);

  function selectContact(id: number) {
    setSelectedGroup(null);
    setSelectedChannel(null);
    setSelected(id);
  }

  function selectGroup(id: number) {
    setSelected(null);
    setSelectedChannel(null);
    setSelectedGroup(id);
  }

  function selectChannel(id: number) {
    setSelected(null);
    setSelectedGroup(null);
    setSelectedChannel(id);
  }

  useEffect(() => {
    refreshContacts();
    refreshGroups();
    refreshChannels();
    refreshStatus();
    // Carried bundles change without events, so the counts are polled too.
    const timer = window.setInterval(refreshStatus, 5000);
    window.addEventListener("focus", markSelectedRead);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", markSelectedRead);
    };
  }, [refreshContacts, refreshGroups, refreshChannels, refreshStatus, markSelectedRead]);

  useEffect(() => {
    if (selected === null && selectedGroup === null && selectedChannel === null && contacts.length > 0) {
      setSelected(contacts[0].id);
    }
  }, [contacts, selected, selectedGroup, selectedChannel]);

  useEffect(() => {
    if (selectedChannel === null) return;
    api.posts(selectedChannel).then(setPosts);
    markSelectedRead();
  }, [selectedChannel, markSelectedRead]);

  useEffect(() => {
    if (selectedGroup === null) return;
    api.groupMessages(selectedGroup).then(setGroupMessages);
    markSelectedRead();
  }, [selectedGroup, markSelectedRead]);

  useEffect(() => {
    if (selected === null) return;
    api.messages(selected).then(setMessages);
    markSelectedRead();
  }, [selected, markSelectedRead]);

  useEffect(() => {
    const stops = [
      api.onEvent((event) => {
        const t = tRef.current;
        if (event.type === "messageReceived") {
          const { message } = event;
          const looking = message.contactId === selectedRef.current && document.hasFocus();
          if (message.contactId === selectedRef.current) {
            setMessages((current) => [...current, message]);
          }
          if (looking) {
            markSelectedRead();
          } else {
            const sender = contactsRef.current.find((contact) => contact.id === message.contactId);
            // A first message from someone new is announced by its request.
            if (sender && notificationsEnabled()) api.notify(sender.name, message.body);
            refreshContacts();
          }
        } else if (event.type === "contactRequest") {
          show(t.wantsToMessage(event.contact.name));
          if (!document.hasFocus() && notificationsEnabled()) api.notify(t.wantsToMessage(event.contact.name), "");
          refreshContacts();
        } else if (event.type === "messageDelivered") {
          setMessages((current) =>
            current.map((message) => (message.id === event.messageId ? { ...message, delivered: true } : message)),
          );
        } else if (event.type === "messagesRead") {
          const read = new Set(event.messageIds);
          setMessages((current) =>
            current.map((message) =>
              read.has(message.id) ? { ...message, delivered: true, read: true } : message,
            ),
          );
        } else if (event.type === "contactChanged") {
          refreshContacts();
        } else if (event.type === "groupMessageReceived") {
          const { message } = event;
          const open = message.groupId === selectedGroupRef.current;
          if (open) setGroupMessages((current) => [...current, message]);
          if (open && document.hasFocus()) {
            markSelectedRead();
          } else {
            const group = groupsRef.current.find((candidate) => candidate.id === message.groupId);
            if (group && notificationsEnabled()) api.notify(`${message.sender} · ${group.name}`, message.body);
            refreshGroups();
          }
        } else if (event.type === "groupChanged") {
          refreshGroups();
          refreshContacts();
        } else if (event.type === "fileChanged") {
          const { file } = event;
          const update = <T extends { file: FileView | null }>(items: T[]) =>
            items.map((item) => (item.file?.id === file.id ? { ...item, file } : item));
          setMessages(update);
          setGroupMessages(update);
          setPosts(update);
        } else if (event.type === "postReceived") {
          const { post } = event;
          const open = post.channelId === selectedChannelRef.current;
          if (open) setPosts((current) => [...current, post]);
          if (open && document.hasFocus()) {
            markSelectedRead();
          } else {
            refreshChannels();
          }
        }
        refreshStatus();
      }),
      api.onError(show),
    ];
    return () => stops.forEach((stop) => stop.then((unlisten) => unlisten()));
  }, [refreshContacts, refreshGroups, refreshChannels, refreshStatus, show, markSelectedRead]);

  const contact = contacts.find((candidate) => candidate.id === selected) ?? null;
  const group = groups.find((candidate) => candidate.id === selectedGroup) ?? null;
  const channel = channels.find((candidate) => candidate.id === selectedChannel) ?? null;

  /** Sends a photo or voice message to whatever is open. */
  async function sendFile(mime: string, bytes: Uint8Array) {
    if (channel) {
      const post = (await api.sendFile({ kind: "channel", id: channel.id }, mime, bytes)) as Post;
      setPosts((current) => [...current, post]);
      refreshChannels();
    } else if (group) {
      const message = (await api.sendFile({ kind: "group", id: group.id }, mime, bytes)) as GroupMessage;
      setGroupMessages((current) => [...current, message]);
      refreshGroups();
    } else if (contact) {
      const message = (await api.sendFile({ kind: "contact", id: contact.id }, mime, bytes)) as Message;
      setMessages((current) => [...current, message]);
      refreshContacts();
    }
    refreshStatus();
  }

  async function sendToChannel(text: string) {
    if (!channel) return;
    const post = await api.sendPost(channel.id, text);
    setPosts((current) => [...current, post]);
    refreshChannels();
    refreshStatus();
  }

  async function mute(post: Post) {
    if (!channel) return;
    const channelId = channel.id;
    try {
      await api.muteAuthor(channelId, post.author);
      setPosts((current) => current.filter((other) => other.author !== post.author));
      show(t.muted(post.nick), { label: t.undo, run: () => unmute(channelId, post.author, post.nick) });
      refreshChannels();
    } catch (error) {
      show(String(error));
    }
  }

  async function unmute(channelId: number, author: string, nick: string) {
    try {
      await api.unmuteAuthor(channelId, author);
      if (selectedChannelRef.current === channelId) api.posts(channelId).then(setPosts);
      refreshChannels();
      show(t.unmuted(nick));
    } catch (error) {
      show(String(error));
    }
  }

  async function addSharedCard(card: string) {
    try {
      const added = await api.addContact(card);
      show(t.memberAdded(added.name));
      await refreshContacts();
      selectContact(added.id);
    } catch (error) {
      show(String(error));
    }
  }

  /** Opens a direct conversation with a group member, adding them as a contact first if needed. */
  async function messageMember(member: Member) {
    let contactId = member.contactId;
    if (contactId === null) {
      const added = await api.addContact(member.card);
      contactId = added.id;
      show(t.memberAdded(added.name));
      await refreshContacts();
      refreshGroups();
    }
    setDialog(null);
    selectContact(contactId);
  }

  async function sendToGroup(text: string) {
    if (!group) return;
    const message = await api.sendGroupMessage(group.id, text);
    setGroupMessages((current) => [...current, message]);
    refreshGroups();
    refreshStatus();
  }

  async function send(text: string) {
    if (!contact) return;
    const message = await api.sendMessage(contact.id, text);
    setMessages((current) => [...current, message]);
    refreshContacts();
    refreshStatus();
  }

  async function accept() {
    if (!contact) return;
    try {
      await api.acceptContact(contact.id);
      refreshContacts();
    } catch (error) {
      show(String(error));
    }
  }

  async function decline() {
    if (!contact) return;
    try {
      await api.declineContact(contact.id);
      setSelected(null);
      setMessages([]);
      refreshContacts();
      show(t.declined(contact.name));
    } catch (error) {
      show(String(error));
    }
  }

  async function exportFile() {
    try {
      const count = await api.exportFile();
      if (count !== null) show(t.exported(count));
    } catch (error) {
      show(String(error));
    }
  }

  async function importFile() {
    try {
      const report = await api.importFile();
      if (report === null) return;
      show(t.imported(report.received, report.delivered, report.carried, report.skipped));
      refreshContacts();
      refreshGroups();
      refreshChannels();
      refreshStatus();
      if (selected !== null) api.messages(selected).then(setMessages);
      if (selectedChannel !== null) api.posts(selectedChannel).then(setPosts);
      if (selectedGroup !== null) api.groupMessages(selectedGroup).then(setGroupMessages);
    } catch (error) {
      show(String(error));
    }
  }

  const requests = contacts.filter((item) => item.request);
  const accepted = contacts.filter((item) => !item.request);

  function contactRow(item: Contact) {
    return (
      <button
        key={item.id}
        className={item.id === selected ? "contact active" : "contact"}
        onClick={() => selectContact(item.id)}
      >
        <span className="avatar" aria-hidden>
          {item.name.slice(0, 1).toLocaleUpperCase(language)}
        </span>
        <span className="who">
          <span className="name">{item.name}</span>
          <span className="preview">{item.lastMessage === null ? t.noMessagesYet : item.lastMessage || t.file}</span>
        </span>
        <span className="meta">
          {item.lastAt !== null && <time>{shortTime(item.lastAt, language)}</time>}
          {item.unread > 0 && item.id !== selected && <span className="badge">{item.unread}</span>}
        </span>
      </button>
    );
  }

  function groupRow(item: Group) {
    return (
      <button
        key={item.id}
        className={item.id === selectedGroup ? "contact active" : "contact"}
        onClick={() => selectGroup(item.id)}
      >
        <span className="avatar group" aria-hidden>
          {item.name.slice(0, 1).toLocaleUpperCase(language)}
        </span>
        <span className="who">
          <span className="name">{item.name}</span>
          <span className="preview">
            {item.lastMessage === null ? t.membersCount(item.members.length) : item.lastMessage || t.file}
          </span>
        </span>
        <span className="meta">
          {item.lastAt !== null && <time>{shortTime(item.lastAt, language)}</time>}
          {item.unread > 0 && item.id !== selectedGroup && <span className="badge">{item.unread}</span>}
        </span>
      </button>
    );
  }

  function channelRow(item: Channel) {
    return (
      <button
        key={item.id}
        className={item.id === selectedChannel ? "contact active" : "contact"}
        onClick={() => selectChannel(item.id)}
      >
        <span className="avatar channel" aria-hidden>
          #
        </span>
        <span className="who">
          <span className="name">{item.name}</span>
          <span className="preview">{item.lastMessage === null ? t.postingAs(item.nick) : item.lastMessage || t.file}</span>
        </span>
        <span className="meta">
          {item.lastAt !== null && <time>{shortTime(item.lastAt, language)}</time>}
          {item.unread > 0 && item.id !== selectedChannel && <span className="badge">{item.unread}</span>}
        </span>
      </button>
    );
  }

  return (
    <div className="shell">
      <aside className="sidebar">
        <header className="brand">
          <Logo size={32} />
          <div>
            <strong>Kervan</strong>
            <span>{profile.name}</span>
          </div>
          <button className="icon-button" onClick={() => setDialog("settings")} aria-label={t.settings} title={t.settings}>
            <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden>
              <path
                fill="currentColor"
                d="M19.4 13a7.6 7.6 0 0 0 0-2l2.1-1.6-2-3.5-2.5 1a7.4 7.4 0 0 0-1.7-1L15 3.3h-4l-.4 2.6a7.4 7.4 0 0 0-1.7 1l-2.5-1-2 3.5L6.5 11a7.6 7.6 0 0 0 0 2l-2.1 1.6 2 3.5 2.5-1a7.4 7.4 0 0 0 1.7 1l.4 2.6h4l.4-2.6a7.4 7.4 0 0 0 1.7-1l2.5 1 2-3.5zM13 15.5a3.5 3.5 0 1 1 0-7 3.5 3.5 0 0 1 0 7z"
                transform="translate(-1 0)"
              />
            </svg>
          </button>
        </header>
        <nav className="contacts" aria-label={t.contacts}>
          {requests.length > 0 && (
            <>
              <h3 className="section-title">{t.requests}</h3>
              {requests.map(contactRow)}
            </>
          )}
          {accepted.length > 0 && (
            <div className="section-row">
              <h3 className="section-title">{t.groups}</h3>
              <button className="icon-button small" onClick={() => setDialog("newGroup")} aria-label={t.newGroup} title={t.newGroup}>
                +
              </button>
            </div>
          )}
          {groups.map(groupRow)}
          <div className="section-row">
            <h3 className="section-title">{t.channels}</h3>
            <button
              className="icon-button small"
              onClick={() => setDialog("joinChannel")}
              aria-label={t.joinChannel}
              title={t.joinChannel}
            >
              +
            </button>
          </div>
          {channels.map(channelRow)}
          {(requests.length > 0 || accepted.length > 0) && <h3 className="section-title">{t.contacts}</h3>}
          {accepted.map(contactRow)}
          {contacts.length === 0 && <p className="sidebar-empty">{t.sidebarEmpty}</p>}
        </nav>
        <footer className="sidebar-actions">
          <button className="primary" onClick={() => setDialog("add")}>
            {t.addContact}
          </button>
          <button onClick={() => setDialog("card")}>{t.myCard}</button>
        </footer>
      </aside>

      <main className="conversation">
        {channel ? (
          <ChannelChat
            channel={channel}
            posts={posts}
            onSend={sendToChannel}
            onSendFile={sendFile}
            onSettings={() => setDialog("channelSettings")}
            onMute={mute}
            onAddCard={addSharedCard}
          />
        ) : group ? (
          <GroupChat
            group={group}
            messages={groupMessages}
            onSend={sendToGroup}
            onSendFile={sendFile}
            onMembers={() => setDialog("members")}
          />
        ) : contact ? (
          <Chat
            contact={contact}
            messages={messages}
            onSend={send}
            onSendFile={sendFile}
            onAccept={accept}
            onDecline={decline}
          />
        ) : (
          <div className="placeholder">
            <Logo size={56} />
            <h2>{t.noConversation}</h2>
            <p>{t.noConversationHint}</p>
          </div>
        )}
      </main>

      <StatusBar
        status={status}
        preview={api.preview}
        onNetworkHelp={() => setDialog("network")}
        onConnect={() => setDialog("connect")}
        onImport={importFile}
        onExport={exportFile}
      />

      {dialog === "add" && (
        <AddContactDialog
          onClose={() => setDialog(null)}
          onAdded={(added) => {
            refreshContacts();
            selectContact(added.id);
          }}
        />
      )}
      {dialog === "newGroup" && (
        <NewGroupDialog
          contacts={contacts}
          onClose={() => setDialog(null)}
          onCreated={(created) => {
            refreshGroups();
            selectGroup(created.id);
            show(t.groupCreated(created.name));
          }}
        />
      )}
      {dialog === "joinChannel" && (
        <JoinChannelDialog
          defaultNick={profile.name}
          onClose={() => setDialog(null)}
          onJoined={(joined) => {
            refreshChannels();
            selectChannel(joined.id);
            show(t.joinedChannel(joined.name));
          }}
        />
      )}
      {dialog === "channelSettings" && channel && (
        <ChannelSettingsDialog
          channel={channel}
          onClose={() => setDialog(null)}
          onChanged={refreshChannels}
          onUnmuted={(nick) => {
            api.posts(channel.id).then(setPosts);
            refreshChannels();
            show(t.unmuted(nick));
          }}
          onShared={() => {
            setDialog(null);
            show(t.cardShared);
            api.posts(channel.id).then(setPosts);
            refreshStatus();
          }}
          onLeft={() => {
            setDialog(null);
            setSelectedChannel(null);
            setPosts([]);
            refreshChannels();
            show(t.leftChannel(channel.name));
          }}
        />
      )}
      {dialog === "members" && group && (
        <MembersDialog
          group={group}
          contacts={contacts}
          onClose={() => setDialog(null)}
          onChanged={() => {
            refreshGroups();
            refreshStatus();
          }}
          onMessage={messageMember}
        />
      )}
      {dialog === "card" && <MyCardDialog profile={profile} onClose={() => setDialog(null)} />}
      {dialog === "settings" && (
        <SettingsDialog
          profile={profile}
          onClose={() => setDialog(null)}
          onRenamed={(renamed) => {
            onProfile(renamed);
            show(t.renamed);
          }}
        />
      )}
      {dialog === "network" && status?.lanProblem && (
        <NetworkDialog problem={status.lanProblem} onClose={() => setDialog(null)} />
      )}
      {dialog === "connect" && (
        <ConnectDialog port={status?.lanPort ?? null} onClose={() => setDialog(null)} onConnected={show} />
      )}
      {notice && (
        <div className="toast" role="status">
          {notice.text}
          {notice.action && (
            <button
              className="toast-action"
              onClick={() => {
                notice.action!.run();
                setNotice(null);
              }}
            >
              {notice.action.label}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

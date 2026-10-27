import {
  BroadcastIcon,
  ChatCircleIcon,
  PencilSimpleIcon,
  UserCircleIcon,
  UserPlusIcon,
  UsersIcon,
  type Icon,
} from "@phosphor-icons/react";
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
  type NearbyPerson,
  type Post,
  type Profile,
  type Status,
} from "./api";
import { useBackStep } from "./back";
import { ChannelChat } from "./ChannelChat";
import { Chat } from "./Chat";
import { ChatList } from "./ChatList";
import {
  AddContactDialog,
  ChannelSettingsDialog,
  ConnectDialog,
  ContactInfoDialog,
  JoinChannelDialog,
  JourneyDialog,
  MembersDialog,
  MyCardDialog,
  NetworkDialog,
  NewDialog,
  NewGroupDialog,
} from "./Dialogs";
import { GroupChat } from "./GroupChat";
import { useI18n } from "./i18n";
import { Logo } from "./Logo";
import { Me } from "./Me";
import { Network } from "./Network";
import { People } from "./People";
import { notificationsEnabled } from "./prefs";
import { Setup } from "./Setup";

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
  | "new"
  | "add"
  | "card"
  | "connect"
  | "network"
  | "contactInfo"
  | "journey"
  | "newGroup"
  | "members"
  | "joinChannel"
  | "channelSettings";

type Tab = "chats" | "people" | "network" | "me";

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
  const [tab, setTab] = useState<Tab>("chats");
  // The message whose journey is open.
  const [journeyId, setJourneyId] = useState<number | null>(null);
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

  // A conversation's history, shown only if that conversation is still open when it arrives: an answer for
  // one left quickly can come after the next one's.
  const loadMessages = useCallback((contactId: number) => {
    api.messages(contactId).then((messages) => {
      if (selectedRef.current === contactId) setMessages(messages);
    });
  }, []);
  const loadGroupMessages = useCallback((groupId: number) => {
    api.groupMessages(groupId).then((messages) => {
      if (selectedGroupRef.current === groupId) setGroupMessages(messages);
    });
  }, []);
  const loadPosts = useCallback((channelId: number) => {
    api.posts(channelId).then((posts) => {
      if (selectedChannelRef.current === channelId) setPosts(posts);
    });
  }, []);
  const channelsRef = useRef(channels);
  channelsRef.current = channels;
  const tRef = useRef(t);
  tRef.current = t;
  const narrow = useNarrow();
  const narrowRef = useRef(narrow);
  narrowRef.current = narrow;

  const refreshContacts = useCallback(() => api.contacts().then(setContacts), []);
  const refreshGroups = useCallback(() => api.groups().then(setGroups), []);
  const refreshChannels = useCallback(() => api.channels().then(setChannels), []);
  const refreshStatus = useCallback(() => api.status().then(setStatus), []);
  // Whether we are visible nearby, and who is, while we are.
  const [nearbyOn, setNearbyOn] = useState<boolean | null>(null);
  const [nearby, setNearby] = useState<NearbyPerson[]>([]);
  const refreshNearby = useCallback(() => api.nearby().then(setNearby), []);

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

  // On a phone the list and the conversation are pages of their own, and
  // opening a conversation is a step that the back button takes back.
  function open() {
    if (narrowRef.current && history.state?.conversation !== true) {
      history.pushState({ conversation: true }, "");
    }
  }

  const closeAll = useCallback(() => {
    setSelected(null);
    setSelectedGroup(null);
    setSelectedChannel(null);
  }, []);

  function back() {
    if (history.state?.conversation === true) history.back();
    else closeAll();
  }

  useEffect(() => {
    window.addEventListener("popstate", closeAll);
    return () => window.removeEventListener("popstate", closeAll);
  }, [closeAll]);

  function selectContact(id: number) {
    open();
    setSelectedGroup(null);
    setSelectedChannel(null);
    setSelected(id);
  }

  function selectGroup(id: number) {
    open();
    setSelected(null);
    setSelectedChannel(null);
    setSelectedGroup(id);
  }

  function selectChannel(id: number) {
    open();
    setSelected(null);
    setSelectedGroup(null);
    setSelectedChannel(id);
  }

  // Android announces messages itself while this page is closed.
  useEffect(() => {
    api.setNotifications(notificationsEnabled(), language);
  }, [language]);

  useEffect(() => {
    api.settings().then((settings) => setNearbyOn(settings.nearby));
    refreshNearby();
  }, [refreshNearby]);

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
    // A phone starts on the list: opening the first contact would hide it.
    if (narrow) return;
    if (selected === null && selectedGroup === null && selectedChannel === null && contacts.length > 0) {
      setSelected(contacts[0].id);
    }
  }, [contacts, selected, selectedGroup, selectedChannel, narrow]);

  useEffect(() => {
    if (selectedChannel === null) return;
    loadPosts(selectedChannel);
    markSelectedRead();
  }, [selectedChannel, markSelectedRead, loadPosts]);

  useEffect(() => {
    if (selectedGroup === null) return;
    loadGroupMessages(selectedGroup);
    markSelectedRead();
  }, [selectedGroup, markSelectedRead, loadGroupMessages]);

  useEffect(() => {
    if (selected === null) return;
    loadMessages(selected);
    markSelectedRead();
  }, [selected, markSelectedRead, loadMessages]);

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
          refreshContacts();
        } else if (event.type === "nearbyChanged") {
          refreshNearby();
        } else if (event.type === "messageMoved") {
          // A message left by a new link: its bubble shows the link.
          if (event.contactId === selectedRef.current) loadMessages(event.contactId);
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
  }, [refreshContacts, refreshGroups, refreshChannels, refreshStatus, refreshNearby, show, markSelectedRead]);

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
      if (selectedChannelRef.current === channelId) loadPosts(channelId);
      refreshChannels();
      show(t.unmuted(nick));
    } catch (error) {
      show(String(error));
    }
  }

  async function toggleNearby(on: boolean) {
    setNearbyOn(on);
    try {
      await api.setNearby(on);
      refreshNearby();
    } catch (error) {
      setNearbyOn(!on);
      show(String(error));
    }
  }

  /** Opens a chat with someone nearby, adding them as a contact first if needed. */
  async function openNearby(person: NearbyPerson) {
    if (person.contactId !== null) {
      selectContact(person.contactId);
      return;
    }
    try {
      const added = await api.addContact(person.card);
      await refreshContacts();
      refreshNearby();
      selectContact(added.id);
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
    return message.id;
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
      back();
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
      if (selected !== null) loadMessages(selected);
      if (selectedChannel !== null) loadPosts(selectedChannel);
      if (selectedGroup !== null) loadGroupMessages(selectedGroup);
    } catch (error) {
      show(String(error));
    }
  }

  const unread = [...contacts, ...groups, ...channels].filter((item) => item.unread > 0 || ("request" in item && item.request)).length;
  const active = channel
    ? { kind: "channel" as const, id: channel.id }
    : group
      ? { kind: "group" as const, id: group.id }
      : contact
        ? { kind: "contact" as const, id: contact.id }
        : null;
  // Android's back button leaves the conversation, then returns to the chats.
  useBackStep(narrow && active !== null, back);
  useBackStep(tab !== "chats", () => setTab("chats"));

  const tabs: { tab: Tab; label: string; icon: Icon; badge?: number }[] = [
    { tab: "chats", label: t.tabChats, icon: ChatCircleIcon, badge: unread },
    { tab: "people", label: t.contacts, icon: UsersIcon },
    { tab: "network", label: t.tabNetwork, icon: BroadcastIcon },
    { tab: "me", label: t.tabMe, icon: UserCircleIcon },
  ];

  return (
    <div className={["shell", narrow && "narrow", active && "open"].filter(Boolean).join(" ")}>
      <aside className="side">
        <div className="side-body">
          {tab === "chats" && (
            <ChatList
              contacts={contacts}
              groups={groups}
              channels={channels}
              status={status}
              active={narrow ? null : active}
              onOpen={(kind, id) =>
                kind === "contact" ? selectContact(id) : kind === "group" ? selectGroup(id) : selectChannel(id)
              }
              nearby={nearbyOn ? nearby : []}
              onNearby={openNearby}
              onNetwork={() => setTab("network")}
              onCard={() => setDialog("card")}
            />
          )}
          {tab === "people" && (
            <People
              contacts={contacts}
              activeId={narrow ? null : selected}
              nearbyOn={nearbyOn}
              nearby={nearby}
              onNearbyOn={toggleNearby}
              onNearby={openNearby}
              onOpen={selectContact}
              onAdd={() => setDialog("add")}
              onCard={() => setDialog("card")}
            />
          )}
          {tab === "network" && (
            <Network
              status={status}
              preview={api.preview}
              onNetworkHelp={() => setDialog("network")}
              onConnect={() => setDialog("connect")}
              onImport={importFile}
              onExport={exportFile}
            />
          )}
          {tab === "me" && (
            <Me
              profile={profile}
              onCard={() => setDialog("card")}
              onRenamed={(renamed) => {
                onProfile(renamed);
                show(t.renamed);
              }}
            />
          )}
        </div>
        {(tab === "chats" || tab === "people") && (
          <button
            className="fab"
            onClick={() => setDialog(tab === "chats" ? "new" : "add")}
            aria-label={tab === "chats" ? t.newTitle : t.addContact}
            title={tab === "chats" ? t.newTitle : t.addContact}
          >
            {tab === "chats" ? <PencilSimpleIcon size={24} weight="bold" /> : <UserPlusIcon size={24} weight="bold" />}
          </button>
        )}
        <nav className="tabbar">
          {tabs.map((item) => {
            const on = item.tab === tab;
            const TabIcon = item.icon;
            return (
              <button
                key={item.tab}
                className={on ? "tab on" : "tab"}
                aria-current={on ? "page" : undefined}
                onClick={() => setTab(item.tab)}
              >
                <span className="pill">
                  <TabIcon size={22} weight={on ? "fill" : "regular"} />
                  {item.badge !== undefined && item.badge > 0 && <span className="tab-badge">{item.badge}</span>}
                </span>
                {item.label}
              </button>
            );
          })}
        </nav>
      </aside>

      <main className="conversation">
        {channel ? (
          <ChannelChat
            channel={channel}
            posts={posts}
            onSend={sendToChannel}
            onSendFile={sendFile}
            onSettings={() => setDialog("channelSettings")}
            onBack={narrow ? back : undefined}
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
            onBack={narrow ? back : undefined}
          />
        ) : contact ? (
          <Chat
            contact={contact}
            messages={messages}
            onSend={send}
            onSendFile={sendFile}
            onAccept={accept}
            onDecline={decline}
            onInfo={() => setDialog("contactInfo")}
            onJourney={(message) => {
              setJourneyId(message.id);
              setDialog("journey");
            }}
            onBack={narrow ? back : undefined}
          />
        ) : (
          <div className="placeholder">
            <Logo size={56} />
            <h2>{t.noConversation}</h2>
            <p>{t.noConversationHint}</p>
          </div>
        )}
      </main>

      {dialog === "new" && (
        <NewDialog onClose={() => setDialog(null)} onChoose={(choice) => setDialog(choice)} />
      )}
      {dialog === "contactInfo" && contact && (
        <ContactInfoDialog contact={contact} onClose={() => setDialog(null)} />
      )}
      {dialog === "journey" && contact && messages.some((message) => message.id === journeyId) && (
        <JourneyDialog
          message={messages.find((message) => message.id === journeyId)!}
          name={contact.name}
          onClose={() => setDialog(null)}
        />
      )}
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
            loadPosts(channel.id);
            refreshChannels();
            show(t.unmuted(nick));
          }}
          onShared={() => {
            setDialog(null);
            show(t.cardShared);
            loadPosts(channel.id);
            refreshStatus();
          }}
          onLeft={() => {
            setDialog(null);
            back();
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

const NARROW = "(max-width: 640px)";

/** Whether the window is phone-sized, kept current as it changes. */
function useNarrow() {
  const [narrow, setNarrow] = useState(() => window.matchMedia(NARROW).matches);
  useEffect(() => {
    const query = window.matchMedia(NARROW);
    const changed = () => setNarrow(query.matches);
    query.addEventListener("change", changed);
    return () => query.removeEventListener("change", changed);
  }, []);
  return narrow;
}

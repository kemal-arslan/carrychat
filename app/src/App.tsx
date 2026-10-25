import { useCallback, useEffect, useRef, useState } from "react";
import { api, type Contact, type Group, type GroupMessage, type Message, type Profile, type Status } from "./api";
import { Chat } from "./Chat";
import {
  AddContactDialog,
  ConnectDialog,
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

type DialogName = "add" | "card" | "connect" | "network" | "settings" | "newGroup" | "members";

function Main({ profile, onProfile }: { profile: Profile; onProfile: (profile: Profile) => void }) {
  const { t, language } = useI18n();
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [groups, setGroups] = useState<Group[]>([]);
  const [selectedGroup, setSelectedGroup] = useState<number | null>(null);
  const [groupMessages, setGroupMessages] = useState<GroupMessage[]>([]);
  const [status, setStatus] = useState<Status | null>(null);
  const [dialog, setDialog] = useState<DialogName | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const contactsRef = useRef(contacts);
  contactsRef.current = contacts;
  const selectedGroupRef = useRef(selectedGroup);
  selectedGroupRef.current = selectedGroup;
  const groupsRef = useRef(groups);
  groupsRef.current = groups;
  const tRef = useRef(t);
  tRef.current = t;

  const refreshContacts = useCallback(() => api.contacts().then(setContacts), []);
  const refreshGroups = useCallback(() => api.groups().then(setGroups), []);
  const refreshStatus = useCallback(() => api.status().then(setStatus), []);

  const show = useCallback((text: string) => {
    setNotice(text);
    window.setTimeout(() => setNotice((current) => (current === text ? null : current)), 5000);
  }, []);

  // Messages count as read only while someone is looking at them.
  const markSelectedRead = useCallback(() => {
    if (!document.hasFocus()) return;
    const contactId = selectedRef.current;
    const groupId = selectedGroupRef.current;
    if (contactId !== null) api.markRead(contactId).then(refreshContacts);
    if (groupId !== null) api.markGroupRead(groupId).then(refreshGroups);
  }, [refreshContacts, refreshGroups]);

  function selectContact(id: number) {
    setSelectedGroup(null);
    setSelected(id);
  }

  function selectGroup(id: number) {
    setSelected(null);
    setSelectedGroup(id);
  }

  useEffect(() => {
    refreshContacts();
    refreshGroups();
    refreshStatus();
    // Carried bundles change without events, so the counts are polled too.
    const timer = window.setInterval(refreshStatus, 5000);
    window.addEventListener("focus", markSelectedRead);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", markSelectedRead);
    };
  }, [refreshContacts, refreshGroups, refreshStatus, markSelectedRead]);

  useEffect(() => {
    if (selected === null && selectedGroup === null && contacts.length > 0) setSelected(contacts[0].id);
  }, [contacts, selected, selectedGroup]);

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
        }
        refreshStatus();
      }),
      api.onError(show),
    ];
    return () => stops.forEach((stop) => stop.then((unlisten) => unlisten()));
  }, [refreshContacts, refreshGroups, refreshStatus, show, markSelectedRead]);

  const contact = contacts.find((candidate) => candidate.id === selected) ?? null;
  const group = groups.find((candidate) => candidate.id === selectedGroup) ?? null;

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
      refreshStatus();
      if (selected !== null) api.messages(selected).then(setMessages);
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
          <span className="preview">{item.lastMessage ?? t.noMessagesYet}</span>
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
          <span className="preview">{item.lastMessage ?? t.membersCount(item.members.length)}</span>
        </span>
        <span className="meta">
          {item.lastAt !== null && <time>{shortTime(item.lastAt, language)}</time>}
          {item.unread > 0 && item.id !== selectedGroup && <span className="badge">{item.unread}</span>}
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
        {group ? (
          <GroupChat group={group} messages={groupMessages} onSend={sendToGroup} onMembers={() => setDialog("members")} />
        ) : contact ? (
          <Chat contact={contact} messages={messages} onSend={send} onAccept={accept} onDecline={decline} />
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
      {dialog === "members" && group && (
        <MembersDialog
          group={group}
          contacts={contacts}
          onClose={() => setDialog(null)}
          onChanged={() => {
            refreshGroups();
            refreshStatus();
          }}
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
          {notice}
        </div>
      )}
    </div>
  );
}

import { useCallback, useEffect, useRef, useState } from "react";
import { api, type Contact, type Message, type Profile, type Status } from "./api";
import { Chat } from "./Chat";
import { AddContactDialog, ConnectDialog, MyCardDialog, NetworkDialog } from "./Dialogs";
import { Logo } from "./Logo";
import { Setup } from "./Setup";
import { StatusBar } from "./StatusBar";
import { plural, shortTime } from "./format";

export default function App() {
  const [profile, setProfile] = useState<Profile | null | undefined>(undefined);

  useEffect(() => {
    api.profile().then(setProfile);
  }, []);

  if (profile === undefined) return null;
  if (profile === null) return <Setup onCreated={setProfile} />;
  return <Main profile={profile} />;
}

type DialogName = "add" | "card" | "connect" | "network";

function Main({ profile }: { profile: Profile }) {
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [unread, setUnread] = useState<Record<number, number>>({});
  const [status, setStatus] = useState<Status | null>(null);
  const [dialog, setDialog] = useState<DialogName | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const selectedRef = useRef(selected);
  selectedRef.current = selected;

  const refreshContacts = useCallback(() => api.contacts().then(setContacts), []);
  const refreshStatus = useCallback(() => api.status().then(setStatus), []);

  const show = useCallback((text: string) => {
    setNotice(text);
    window.setTimeout(() => setNotice((current) => (current === text ? null : current)), 5000);
  }, []);

  useEffect(() => {
    refreshContacts().then(() => undefined);
    refreshStatus();
    // Carried bundles change without events, so the counts are polled too.
    const timer = window.setInterval(refreshStatus, 5000);
    return () => window.clearInterval(timer);
  }, [refreshContacts, refreshStatus]);

  useEffect(() => {
    if (selected === null && contacts.length > 0) setSelected(contacts[0].id);
  }, [contacts, selected]);

  useEffect(() => {
    if (selected === null) return;
    api.messages(selected).then(setMessages);
    setUnread((counts) => ({ ...counts, [selected]: 0 }));
  }, [selected]);

  useEffect(() => {
    const stops = [
      api.onEvent((event) => {
        if (event.type === "messageReceived") {
          const { message } = event;
          if (message.contactId === selectedRef.current) {
            setMessages((current) => [...current, message]);
          } else {
            setUnread((counts) => ({ ...counts, [message.contactId]: (counts[message.contactId] ?? 0) + 1 }));
          }
          refreshContacts();
        } else if (event.type === "contactRequest") {
          show(`${event.contact.name} wants to message you.`);
          refreshContacts();
        } else if (event.type === "messageDelivered") {
          setMessages((current) =>
            current.map((message) => (message.id === event.messageId ? { ...message, delivered: true } : message)),
          );
        }
        refreshStatus();
      }),
      api.onError(show),
    ];
    return () => stops.forEach((stop) => stop.then((unlisten) => unlisten()));
  }, [refreshContacts, refreshStatus, show]);

  const contact = contacts.find((candidate) => candidate.id === selected) ?? null;

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
      show(`Declined ${contact.name}. Their messages are deleted and new ones will be ignored.`);
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
        onClick={() => setSelected(item.id)}
      >
        <span className="avatar" aria-hidden>
          {item.name.slice(0, 1).toUpperCase()}
        </span>
        <span className="who">
          <span className="name">{item.name}</span>
          <span className="preview">{item.lastMessage ?? "No messages yet"}</span>
        </span>
        <span className="meta">
          {item.lastAt !== null && <time>{shortTime(item.lastAt)}</time>}
          {(unread[item.id] ?? 0) > 0 && <span className="badge">{unread[item.id]}</span>}
        </span>
      </button>
    );
  }

  async function exportFile() {
    try {
      const count = await api.exportFile();
      if (count !== null) show(`Saved ${plural(count, "item", "items")} to the file. Carry it to another Kervan computer.`);
    } catch (error) {
      show(String(error));
    }
  }

  async function importFile() {
    try {
      const report = await api.importFile();
      if (report === null) return;
      const parts = [
        plural(report.received, "new message", "new messages"),
        plural(report.delivered, "delivery receipt", "delivery receipts"),
        `${report.carried} to carry for others`,
      ];
      if (report.skipped > 0) parts.push(`${report.skipped} unreadable`);
      show(`Imported: ${parts.join(", ")}.`);
      refreshContacts();
      refreshStatus();
      if (selected !== null) api.messages(selected).then(setMessages);
    } catch (error) {
      show(String(error));
    }
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
        </header>
        <nav className="contacts" aria-label="Contacts">
          {requests.length > 0 && (
            <>
              <h3 className="section-title">Requests</h3>
              {requests.map(contactRow)}
              <h3 className="section-title">Contacts</h3>
            </>
          )}
          {accepted.map(contactRow)}
          {contacts.length === 0 && (
            <p className="sidebar-empty">
              Add the people you want to reach. Only one of you needs the other's card: your first message carries
              yours.
            </p>
          )}
        </nav>
        <footer className="sidebar-actions">
          <button className="primary" onClick={() => setDialog("add")}>
            Add contact
          </button>
          <button onClick={() => setDialog("card")}>My card</button>
        </footer>
      </aside>

      <main className="conversation">
        {contact ? (
          <Chat contact={contact} messages={messages} onSend={send} onAccept={accept} onDecline={decline} />
        ) : (
          <div className="placeholder">
            <Logo size={56} />
            <h2>No conversation yet</h2>
            <p>Give your card to someone and add theirs. Messages then wait on this computer until a path appears.</p>
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
            setSelected(added.id);
          }}
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
          {notice}
        </div>
      )}
    </div>
  );
}

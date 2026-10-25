// Everything the window asks of the node. Inside the desktop app this calls
// the Rust commands; in a plain browser (`npm run dev`) it serves sample data,
// so the interface can be worked on without building the app.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

export type Profile = { name: string; fingerprint: string; card: string };

export type Contact = {
  id: number;
  name: string;
  fingerprint: string;
  lastMessage: string | null;
  lastAt: number | null;
  /** Wrote first and waits to be accepted or declined. */
  request: boolean;
  /** Messages from them not read yet. */
  unread: number;
};

export type Message = {
  id: number;
  contactId: number;
  outgoing: boolean;
  body: string;
  createdAt: number;
  delivered: boolean;
  /** Outgoing: they read it. Incoming: we read it. */
  read: boolean;
};

export type Settings = { readReceipts: boolean };

/** Why the local network link is not working. `blocked`: macOS denies the Local Network permission. */
export type LanProblem = { kind: "blocked" | "failed"; detail: string };

export type Status = {
  lanPeers: number;
  lanPort: number | null;
  lanProblem: LanProblem | null;
  internetOn: boolean;
  internetRelays: number;
  /** The last reason a relay gave for refusing an event. */
  internetRefusal: string | null;
  outbox: number;
  carried: number;
};

export type ImportReport = {
  received: number;
  delivered: number;
  carried: number;
  skipped: number;
};

export type NodeEvent =
  | { type: "messageReceived"; message: Message }
  | { type: "messageDelivered"; contactId: number; messageId: number }
  | { type: "messagesRead"; contactId: number; messageIds: number[] }
  | { type: "contactChanged"; contact: Contact }
  | { type: "peersChanged"; count: number }
  | { type: "contactRequest"; contact: Contact }
  | { type: "localNetworkChanged"; problem: LanProblem | null }
  | { type: "internetChanged"; relays: number };

export interface Api {
  /** Whether this is the browser preview with sample data. */
  preview: boolean;
  profile(): Promise<Profile | null>;
  createProfile(name: string): Promise<Profile>;
  rename(name: string): Promise<Profile>;
  contacts(): Promise<Contact[]>;
  addContact(card: string): Promise<Contact>;
  acceptContact(contactId: number): Promise<Contact>;
  declineContact(contactId: number): Promise<void>;
  messages(contactId: number): Promise<Message[]>;
  sendMessage(contactId: number, text: string): Promise<Message>;
  markRead(contactId: number): Promise<void>;
  settings(): Promise<Settings>;
  setReadReceipts(on: boolean): Promise<void>;
  status(): Promise<Status>;
  /** Asks where to save, then exports. Returns null if cancelled. */
  exportFile(): Promise<number | null>;
  /** Asks which file, then imports it. Returns null if cancelled. */
  importFile(): Promise<ImportReport | null>;
  connectPeer(address: string): Promise<void>;
  /** Opens the Local Network permission in macOS System Settings. */
  openNetworkSettings(): Promise<void>;
  /** Deletes identity, contacts and messages, and restarts empty. */
  eraseEverything(): Promise<void>;
  /** Shows a system notification, asking for permission the first time. */
  notify(title: string, body: string): Promise<void>;
  onEvent(handler: (event: NodeEvent) => void): Promise<() => void>;
  onError(handler: (message: string) => void): Promise<() => void>;
}

const FILE_FILTERS = [{ name: "Kervan", extensions: ["kervan"] }];

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw new Error(String(error));
  }
}

function desktopApi(): Api {
  return {
    preview: false,
    profile: () => call("profile"),
    createProfile: (name) => call("create_profile", { name }),
    rename: (name) => call("rename", { name }),
    contacts: () => call("contacts"),
    addContact: (card) => call("add_contact", { card }),
    acceptContact: (contactId) => call("accept_contact", { contactId }),
    declineContact: (contactId) => call("decline_contact", { contactId }),
    messages: (contactId) => call("messages", { contactId }),
    sendMessage: (contactId, text) => call("send_message", { contactId, text }),
    markRead: (contactId) => call("mark_read", { contactId }),
    settings: () => call("settings"),
    setReadReceipts: (on) => call("set_read_receipts", { on }),
    status: () => call("status"),
    async exportFile() {
      const day = new Date().toISOString().slice(0, 10);
      const path = await save({ defaultPath: `kervan-${day}.kervan`, filters: FILE_FILTERS });
      return path ? call<number>("export_file", { path }) : null;
    },
    async importFile() {
      const path = await open({ multiple: false, directory: false, filters: FILE_FILTERS });
      return path ? call<ImportReport>("import_file", { path }) : null;
    },
    connectPeer: (address) => call("connect_peer", { address }),
    openNetworkSettings: () => call("open_network_settings"),
    eraseEverything: () => call("erase_everything"),
    async notify(title, body) {
      let granted = await isPermissionGranted();
      if (!granted) granted = (await requestPermission()) === "granted";
      if (granted) sendNotification({ title, body });
    },
    onEvent: (handler) => listen<NodeEvent>("kervan-event", (event) => handler(event.payload)),
    onError: (handler) => listen<string>("kervan-error", (event) => handler(event.payload)),
  };
}

function previewApi(): Api {
  const now = () => Math.floor(Date.now() / 1000);
  const listeners: ((event: NodeEvent) => void)[] = [];
  const emit = (event: NodeEvent) => listeners.forEach((listener) => listener(event));
  let profile: Profile | null = null;
  let settings: Settings = { readReceipts: true };
  let nextId = 10;
  const contact = (id: number, name: string, fingerprint: string, request = false): Contact => ({
    id,
    name,
    fingerprint,
    lastMessage: null,
    lastAt: null,
    request,
    unread: 0,
  });
  const contacts: Contact[] = [
    contact(1, "Deniz", "7jgi 54zm 2zkd yent vzen s3pg lq"),
    contact(2, "Ayşe", "gp6m vxkt dnvv lqmw ewpx qfox be"),
    contact(3, "Mehmet", "fwtj 6jv4 jq4d zgfb o2vz bv72 kq", true),
  ];
  const message = (id: number, contactId: number, outgoing: boolean, body: string, ago: number, done = true): Message => ({
    id,
    contactId,
    outgoing,
    body,
    createdAt: now() - ago,
    delivered: outgoing && done,
    read: done,
  });
  const messages: Message[] = [
    message(1, 1, false, "Elektrik geldi mi sizde?", 5400),
    message(2, 1, true, "Geldi, ama internet hâlâ yok. Su var mı?", 5100),
    message(3, 1, false, "Var, yarın sabah okulun önünde dağıtım olacak.", 4900),
    message(4, 1, true, "Tamam, oraya gelirim. Annem de yanımda.", 600, false),
    message(5, 2, true, "İyiyim, Kadıköy'deyim.", 86400),
    message(7, 2, false, "Çok sevindim. Akşam ararım.", 80000, false),
    message(6, 3, false, "Merhaba, ben Mehmet. Kartını Deniz'den aldım.", 300, false),
  ];
  const summarize = () =>
    contacts.map((contact) => {
      const own = messages.filter((message) => message.contactId === contact.id);
      const last = own.at(-1);
      return {
        ...contact,
        lastMessage: last?.body ?? null,
        lastAt: last?.createdAt ?? null,
        unread: own.filter((message) => !message.outgoing && !message.read).length,
      };
    });
  return {
    preview: true,
    async profile() {
      return profile;
    },
    async createProfile(name) {
      profile = {
        name,
        fingerprint: "b3oi uvmj hsk3 wflg ofyx lmwu ci",
        card: "kervan:ANBKsjJ0K7SrOhNovUYV5ObQIkq3GgFrr4UgozLJd4c3BSpQdzrI2Rdz8tyWYuEvDe_pFeQVuKHI4gpaPWqyuEMFYWxpY2XdVELCbsBqOz0qO_dKG4ggm5THqxr3I5G2SZI9XsHtAcZk67tdtelzFtx3SYKB-TSNiDYR9ZlQBS4bMi8v4aIF",
      };
      return profile;
    },
    async rename(name) {
      profile = { ...profile!, name: name.trim() };
      return profile;
    },
    async contacts() {
      return summarize();
    },
    async addContact(card) {
      if (!card.trim().startsWith("kervan:")) throw new Error("invalid card prefix");
      const added = contact(nextId++, "Elif", "q3rd 7kpa mx2c vuhe 4tzn 6wyb fo");
      contacts.push(added);
      return added;
    },
    async acceptContact(contactId) {
      const found = contacts.find((candidate) => candidate.id === contactId)!;
      found.request = false;
      return found;
    },
    async declineContact(contactId) {
      contacts.splice(contacts.findIndex((candidate) => candidate.id === contactId), 1);
    },
    async messages(contactId) {
      return messages.filter((message) => message.contactId === contactId).map((message) => ({ ...message }));
    },
    async sendMessage(contactId, text) {
      const sent = { ...message(nextId++, contactId, true, text.trim(), 0, false) };
      messages.push(sent);
      setTimeout(() => {
        sent.delivered = true;
        emit({ type: "messageDelivered", contactId, messageId: sent.id });
      }, 2000);
      setTimeout(() => {
        sent.read = true;
        emit({ type: "messagesRead", contactId, messageIds: [sent.id] });
      }, 4000);
      return { ...sent };
    },
    async markRead(contactId) {
      messages.filter((message) => message.contactId === contactId && !message.outgoing).forEach((message) => {
        message.read = true;
      });
    },
    async settings() {
      return settings;
    },
    async setReadReceipts(on) {
      settings = { readReceipts: on };
    },
    async status() {
      // ?lan=blocked or ?lan=failed shows the local network warnings.
      const problem = new URLSearchParams(location.search).get("lan");
      const lanProblem: LanProblem | null =
        problem === "blocked"
          ? { kind: "blocked", detail: "Could not announce this computer: No route to host (os error 65)" }
          : problem === "failed"
            ? { kind: "failed", detail: "Found 192.168.1.5:55007 but could not connect: Connection refused (os error 61)" }
            : null;
      return {
        lanPeers: lanProblem ? 0 : 1,
        lanPort: 49333,
        lanProblem,
        internetOn: true,
        internetRelays: 3,
        internetRefusal: null,
        outbox: messages.filter((m) => m.outgoing && !m.delivered).length,
        carried: 3,
      };
    },
    async exportFile() {
      return 4;
    },
    async importFile() {
      return { received: 1, delivered: 1, carried: 2, skipped: 0 };
    },
    async connectPeer() {},
    async openNetworkSettings() {},
    async eraseEverything() {
      location.reload();
    },
    async notify() {},
    async onEvent(handler) {
      listeners.push(handler);
      return () => listeners.splice(listeners.indexOf(handler), 1);
    },
    async onError() {
      return () => {};
    },
  };
}

export const api: Api = "__TAURI_INTERNALS__" in window ? desktopApi() : previewApi();

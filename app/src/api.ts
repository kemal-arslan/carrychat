// Everything the window asks of the node. Inside the desktop app this calls
// the Rust commands; in a plain browser (`npm run dev`) it serves sample data,
// so the interface can be worked on without building the app.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";

export type Profile = { name: string; fingerprint: string; card: string };

export type Contact = {
  id: number;
  name: string;
  fingerprint: string;
  lastMessage: string | null;
  lastAt: number | null;
  /** Wrote first and waits to be accepted or declined. */
  request: boolean;
};

export type Message = {
  id: number;
  contactId: number;
  outgoing: boolean;
  body: string;
  createdAt: number;
  delivered: boolean;
};

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
  | { type: "peersChanged"; count: number }
  | { type: "contactRequest"; contact: Contact }
  | { type: "localNetworkChanged"; problem: LanProblem | null }
  | { type: "internetChanged"; relays: number };

export interface Api {
  /** Whether this is the browser preview with sample data. */
  preview: boolean;
  profile(): Promise<Profile | null>;
  createProfile(name: string): Promise<Profile>;
  contacts(): Promise<Contact[]>;
  addContact(card: string): Promise<Contact>;
  acceptContact(contactId: number): Promise<Contact>;
  declineContact(contactId: number): Promise<void>;
  messages(contactId: number): Promise<Message[]>;
  sendMessage(contactId: number, text: string): Promise<Message>;
  status(): Promise<Status>;
  /** Asks where to save, then exports. Returns null if cancelled. */
  exportFile(): Promise<number | null>;
  /** Asks which file, then imports it. Returns null if cancelled. */
  importFile(): Promise<ImportReport | null>;
  connectPeer(address: string): Promise<void>;
  /** Opens the Local Network permission in macOS System Settings. */
  openNetworkSettings(): Promise<void>;
  onEvent(handler: (event: NodeEvent) => void): Promise<() => void>;
  onError(handler: (message: string) => void): Promise<() => void>;
}

const FILE_FILTERS = [{ name: "Kervan messages", extensions: ["kervan"] }];

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
    contacts: () => call("contacts"),
    addContact: (card) => call("add_contact", { card }),
    acceptContact: (contactId) => call("accept_contact", { contactId }),
    declineContact: (contactId) => call("decline_contact", { contactId }),
    messages: (contactId) => call("messages", { contactId }),
    sendMessage: (contactId, text) => call("send_message", { contactId, text }),
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
    onEvent: (handler) => listen<NodeEvent>("kervan-event", (event) => handler(event.payload)),
    onError: (handler) => listen<string>("kervan-error", (event) => handler(event.payload)),
  };
}

function previewApi(): Api {
  const now = () => Math.floor(Date.now() / 1000);
  const listeners: ((event: NodeEvent) => void)[] = [];
  const emit = (event: NodeEvent) => listeners.forEach((listener) => listener(event));
  let profile: Profile | null = null;
  let nextId = 10;
  const contacts: Contact[] = [
    { id: 1, name: "Deniz", fingerprint: "7jgi 54zm 2zkd yent vzen s3pg lq", lastMessage: null, lastAt: null, request: false },
    { id: 2, name: "Ayşe", fingerprint: "gp6m vxkt dnvv lqmw ewpx qfox be", lastMessage: null, lastAt: null, request: false },
    { id: 3, name: "Mehmet", fingerprint: "fwtj 6jv4 jq4d zgfb o2vz bv72 kq", lastMessage: null, lastAt: null, request: true },
  ];
  const messages: Message[] = [
    { id: 1, contactId: 1, outgoing: false, body: "Elektrik geldi mi sizde?", createdAt: now() - 5400, delivered: false },
    { id: 2, contactId: 1, outgoing: true, body: "Geldi, ama internet hâlâ yok. Su var mı?", createdAt: now() - 5100, delivered: true },
    { id: 3, contactId: 1, outgoing: false, body: "Var, yarın sabah okulun önünde dağıtım olacak.", createdAt: now() - 4900, delivered: false },
    { id: 4, contactId: 1, outgoing: true, body: "Tamam, oraya gelirim. Annem de yanımda.", createdAt: now() - 600, delivered: false },
    { id: 5, contactId: 2, outgoing: true, body: "İyiyim, Kadıköy'deyim.", createdAt: now() - 86400, delivered: true },
    { id: 6, contactId: 3, outgoing: false, body: "Merhaba, ben Mehmet. Kartını Deniz'den aldım.", createdAt: now() - 300, delivered: false },
  ];
  const summarize = () =>
    contacts.map((contact) => {
      const last = messages.filter((message) => message.contactId === contact.id).at(-1);
      return { ...contact, lastMessage: last?.body ?? null, lastAt: last?.createdAt ?? null };
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
    async contacts() {
      return summarize();
    },
    async addContact(card) {
      if (!card.trim().startsWith("kervan:")) throw new Error("invalid card prefix");
      const contact = { id: nextId++, name: "Elif", fingerprint: "q3rd 7kpa mx2c vuhe 4tzn 6wyb fo", lastMessage: null, lastAt: null, request: false };
      contacts.push(contact);
      return contact;
    },
    async acceptContact(contactId) {
      const contact = contacts.find((candidate) => candidate.id === contactId)!;
      contact.request = false;
      return contact;
    },
    async declineContact(contactId) {
      contacts.splice(contacts.findIndex((candidate) => candidate.id === contactId), 1);
    },
    async messages(contactId) {
      return messages.filter((message) => message.contactId === contactId);
    },
    async sendMessage(contactId, text) {
      const message = { id: nextId++, contactId, outgoing: true, body: text.trim(), createdAt: now(), delivered: false };
      messages.push(message);
      setTimeout(() => {
        message.delivered = true;
        emit({ type: "messageDelivered", contactId, messageId: message.id });
      }, 2500);
      return message;
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

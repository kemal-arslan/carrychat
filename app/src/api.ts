// Everything the window asks of the node. Inside the desktop app this calls
// the Rust commands; in a plain browser (`npm run dev`) it serves sample data,
// so the interface can be worked on without building the app.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

export type Profile = { name: string; fingerprint: string; card: string };

/** A card heard over sound, its signature verified. */
export type HeardCard = { card: string; name: string };

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
  /** Our messages to them that have not arrived yet. */
  waiting: number;
  /** A ratchet session runs both ways, so messages have forward secrecy. */
  secure: boolean;
  /** They have our card, so they can join our groups. */
  hasOurCard: boolean;
};

export type Member = {
  name: string;
  fingerprint: string;
  me: boolean;
  admin: boolean;
  /** Their contact id, if they are one of our contacts. */
  contactId: number | null;
  /** Their card as the group's admin passed it on, to add them as a contact. */
  card: string;
};

export type Group = {
  id: number;
  name: string;
  /** We are its admin, who alone changes its members. */
  admin: boolean;
  /** We are still in it. */
  member: boolean;
  members: Member[];
  lastMessage: string | null;
  lastAt: number | null;
  unread: number;
};

export type Channel = {
  id: number;
  /** Normalized, without "#". */
  name: string;
  hasPassword: boolean;
  /** The name we post under. */
  nick: string;
  lastMessage: string | null;
  lastAt: number | null;
  unread: number;
};

export type SharedCard = { name: string; fingerprint: string; card: string };

/** An author muted in a channel. */
export type Muted = { author: string; nick: string };

export type Post = {
  id: number;
  channelId: number;
  outgoing: boolean;
  nick: string;
  /** A short id of the author's key, to tell apart authors with one nickname. */
  author: string;
  /** The text, empty for a shared card. */
  body: string;
  sharedCard: SharedCard | null;
  createdAt: number;
  read: boolean;
  file: FileView | null;
};

/** A photo or voice message offered in a message, and how much of it is here. */
export type FileView = {
  id: number;
  mime: string;
  size: number;
  chunks: number;
  received: number;
  ready: boolean;
  /** After this, missing parts will not come any more. */
  expiresAt: number;
};

export type FileTarget = { kind: "contact" | "group" | "channel"; id: number };

export type GroupMessage = {
  id: number;
  groupId: number;
  outgoing: boolean;
  /** The sender's name, for messages from other members. */
  sender: string | null;
  body: string;
  createdAt: number;
  read: boolean;
  file: FileView | null;
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
  file: FileView | null;
  /** Outgoing: the links it left this device by so far. */
  via: Via[];
};

/** A way a message left this device. */
export type Via = "lan" | "bluetooth" | "wifiAware" | "wifiDirect" | "internet" | "lora" | "reticulum" | "file";

/** A link a nearby node is connected over. */
export type Link = "lan" | "bluetooth" | "wifiAware" | "wifiDirect";

/** Someone nearby who is visible, as we are. */
export type NearbyPerson = {
  /** Their node's session id, steady while they stay. */
  session: string;
  name: string;
  fingerprint: string;
  /** Their card, to add them as a contact. */
  card: string;
  link: Link;
  /** Set when they are one of our contacts already. */
  contactId: number | null;
};

/** How Wi-Fi Aware, or Wi-Fi Direct, stands on this phone; null where there is none. */
export type WifiAware = "unsupported" | "off" | "on" | null;

/** One of our messages from writing to reading. */
export type Journey = {
  createdAt: number;
  deliveredAt: number | null;
  readAt: number | null;
  /** Each link it left by, in the order first used: how many devices or relays took it, and when. */
  steps: { via: Via; count: number; at: number }[];
};

export type Settings = {
  readReceipts: boolean;
  tor: boolean;
  bluetooth: boolean;
  nearby: boolean;
  /** The Meshtastic or MeshCore device chosen for LoRa: `tcp:host:port`, a serial port or `bluetooth:address`. */
  loraDevice: string | null;
  /** The Reticulum node to connect to, as `host:port`. */
  reticulum: string | null;
};

/** A device the LoRa device heard. `at` is in seconds since the epoch. */
export type LoraNode = { name: string; at: number; snr: number | null; hops: number | null };

/**
 * How the LoRa link stands. A Meshtastic device has a `region`, and "UNSET" means it does not transmit; a
 * MeshCore device has a `radio` setting, and `noFreeChannel` when CarryChat's channel could not be added.
 * `heard` covers the last `heardWithin` seconds.
 */
export type LoraStatus = {
  device: string | null;
  connected: boolean;
  firmware: "Meshtastic" | "MeshCore" | null;
  name: string | null;
  region: string | null;
  radio: string | null;
  noFreeChannel: boolean;
  waiting: number;
  heard: LoraNode[];
  heardWithin: number;
  carrychatAt: number | null;
};

/** A CarryChat node heard on Reticulum: its destination in hex, how many hops away, and when it last announced. */
export type ReticulumPeer = { destination: string; hops: number; at: number };

/** How the Reticulum link stands: the node connected to, and the CarryChat nodes heard through it. */
export type ReticulumStatus = {
  address: string | null;
  connected: boolean;
  /** This node's own destination, in hex. */
  destination: string | null;
  peers: ReticulumPeer[];
  waiting: number;
  carrychatAt: number | null;
};

/** A USB serial port a LoRa device may be on. */
export type LoraPort = { path: string; label: string | null };

/** A Meshtastic or MeshCore device found over Bluetooth. */
export type FoundDevice = { address: string; name: string | null };

/** How the internet link stands with Tor, when it goes over Tor. */
export type TorState = { state: "starting" } | { state: "ready" } | { state: "failed"; detail: string };

/** Why the local network link is not working. `blocked`: macOS denies the Local Network permission. */
export type LanProblem = { kind: "blocked" | "failed"; detail: string };

export type Status = {
  lanPeers: number;
  lanPort: number | null;
  lanProblem: LanProblem | null;
  bluetoothOn: boolean;
  bluetoothPeers: number;
  /** Whether this device's Bluetooth is on; null until the app knows. */
  bluetoothRadio: boolean | null;
  wifiAwarePeers: number;
  wifiDirectPeers: number;
  /** Whether a LoRa device is connected. */
  loraConnected: boolean;
  /** Frames waiting for airtime on LoRa. */
  loraWaiting: number;
  internetOn: boolean;
  internetRelays: number;
  /** The last reason a relay gave for refusing an event. */
  internetRefusal: string | null;
  /** Tor's state, when the internet link goes over Tor. */
  internetTor: TorState | null;
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
  | { type: "messageMoved"; contactId: number; messageId: number }
  | { type: "messagesRead"; contactId: number; messageIds: number[] }
  | { type: "contactChanged"; contact: Contact }
  | { type: "peersChanged"; lan: number; bluetooth: number; wifiAware: number; wifiDirect: number }
  | { type: "loraChanged"; connected: boolean }
  | { type: "nearbyChanged"; people: number }
  | { type: "contactRequest"; contact: Contact }
  | { type: "localNetworkChanged"; problem: LanProblem | null }
  | { type: "internetChanged"; relays: number }
  | { type: "groupMessageReceived"; message: GroupMessage }
  | { type: "groupChanged"; group: Group }
  | { type: "postReceived"; post: Post }
  | { type: "fileChanged"; file: FileView };

export interface Api {
  /** Whether this is the browser preview with sample data. */
  preview: boolean;
  profile(): Promise<Profile | null>;
  createProfile(name: string): Promise<Profile>;
  rename(name: string): Promise<Profile>;
  contacts(): Promise<Contact[]>;
  addContact(card: string): Promise<Contact>;
  /** This device's card as sound: one round, a WAV file to play over and over. */
  cardSound(): Promise<ArrayBuffer>;
  /** Starts listening for a card played as sound, from a microphone at `rate` samples a second. */
  listenStart(rate: number): Promise<void>;
  /** The microphone's next samples, 16-bit; gives the card once it has been heard whole. */
  listenHeard(samples: Uint8Array): Promise<HeardCard | null>;
  listenStop(): Promise<void>;
  /** A card read over NFC, in its text form, once its signature verifies. */
  readCard(card: string): Promise<HeardCard>;
  acceptContact(contactId: number): Promise<Contact>;
  declineContact(contactId: number): Promise<void>;
  messages(contactId: number): Promise<Message[]>;
  /** Where one of our messages went. */
  journey(messageId: number): Promise<Journey>;
  sendMessage(contactId: number, text: string): Promise<Message>;
  markRead(contactId: number): Promise<void>;
  groups(): Promise<Group[]>;
  groupMessages(groupId: number): Promise<GroupMessage[]>;
  createGroup(name: string, contactIds: number[]): Promise<Group>;
  addGroupMember(groupId: number, contactId: number): Promise<Group>;
  removeGroupMember(groupId: number, fingerprint: string): Promise<Group>;
  leaveGroup(groupId: number): Promise<Group>;
  sendGroupMessage(groupId: number, text: string): Promise<GroupMessage>;
  markGroupRead(groupId: number): Promise<void>;
  channels(): Promise<Channel[]>;
  joinChannel(name: string, password: string, nick: string): Promise<Channel>;
  leaveChannel(channelId: number): Promise<void>;
  setChannelNick(channelId: number, nick: string): Promise<Channel>;
  posts(channelId: number): Promise<Post[]>;
  sendPost(channelId: number, text: string): Promise<Post>;
  shareCard(channelId: number): Promise<Post>;
  muteAuthor(channelId: number, author: string): Promise<void>;
  unmuteAuthor(channelId: number, author: string): Promise<void>;
  mutedAuthors(channelId: number): Promise<Muted[]>;
  markChannelRead(channelId: number): Promise<void>;
  /** Sends a photo or voice message; returns the message that offers it. */
  sendFile(target: FileTarget, mime: string, bytes: Uint8Array): Promise<Message | GroupMessage | Post>;
  /** An object URL for a file that is all here, or null. */
  fileUrl(fileId: number): Promise<string | null>;
  /** Asks where to save a file that arrived, then saves it. False if cancelled. */
  saveFile(file: FileView): Promise<boolean>;
  settings(): Promise<Settings>;
  setReadReceipts(on: boolean): Promise<void>;
  /** Chooses Tor for the internet link; it applies after a restart. */
  setTor(on: boolean): Promise<void>;
  /** Chooses whether the app uses Bluetooth; it applies after a restart. */
  setBluetooth(on: boolean): Promise<void>;
  /** Becomes visible to people nearby who are visible too, or stops. */
  setNearby(on: boolean): Promise<void>;
  /** The people nearby who are visible, while we are. */
  nearby(): Promise<NearbyPerson[]>;
  wifiAware(): Promise<WifiAware>;
  wifiDirect(): Promise<WifiAware>;
  lora(): Promise<LoraStatus>;
  setLora(device: string | null): Promise<void>;
  loraPorts(): Promise<LoraPort[]>;
  /** Starts looking for Meshtastic and MeshCore devices over Bluetooth, on a phone. */
  loraScan(): Promise<void>;
  loraFound(): Promise<FoundDevice[]>;
  reticulum(): Promise<ReticulumStatus>;
  /** Connects to a Reticulum node, as `host:port`, or to none. */
  setReticulum(address: string | null): Promise<void>;
  restart(): Promise<void>;
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
  /** Tells the app the notification setting and language, which Android
   * uses for messages that arrive while this page is closed. */
  setNotifications(on: boolean, language: string): Promise<void>;
  onEvent(handler: (event: NodeEvent) => void): Promise<() => void>;
  onError(handler: (message: string) => void): Promise<() => void>;
}

const FILE_FILTERS = [{ name: "CarryChat", extensions: ["carrychat"] }];

function torPreview(): TorState | null {
  const state = new URLSearchParams(location.search).get("tor");
  if (state === "starting" || state === "ready") return { state };
  if (state === "failed") return { state, detail: "Unable to connect to the Tor network" };
  return null;
}

/** A name to suggest when saving a file, from its media type. */
function fileName(file: FileView): string {
  const extension: Record<string, string> = {
    "image/jpeg": "jpg",
    "image/png": "png",
    "image/webp": "webp",
    "audio/mp4": "m4a",
    "audio/webm": "webm",
    "audio/ogg": "ogg",
  };
  return `carrychat-${file.id}.${extension[file.mime] ?? "bin"}`;
}

/** Object URLs of files that arrived, made once each. */
const fileUrls = new Map<number, string>();

function toBase64(bytes: Uint8Array): string {
  let text = "";
  for (let start = 0; start < bytes.length; start += 0x8000) {
    text += String.fromCharCode(...bytes.subarray(start, start + 0x8000));
  }
  return btoa(text);
}

function fromBase64(text: string): Uint8Array<ArrayBuffer> {
  const raw = atob(text);
  const bytes = new Uint8Array(raw.length);
  for (let index = 0; index < raw.length; index++) bytes[index] = raw.charCodeAt(index);
  return bytes;
}

async function call<T>(command: string, args?: Record<string, unknown> | Uint8Array): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw new Error(String(error));
  }
}

/** When the preview started listening for a card. */
let listeningSince = 0;

/** Half a second of 2 kHz, as a WAV file, for the preview's card sound. */
function beep(): ArrayBuffer {
  const rate = 48_000;
  const samples = rate / 2;
  const view = new DataView(new ArrayBuffer(44 + samples * 2));
  const text = (at: number, value: string) => [...value].forEach((char, index) => view.setUint8(at + index, char.charCodeAt(0)));
  text(0, "RIFF");
  view.setUint32(4, 36 + samples * 2, true);
  text(8, "WAVEfmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  text(36, "data");
  view.setUint32(40, samples * 2, true);
  for (let index = 0; index < samples; index++) {
    view.setInt16(44 + index * 2, Math.sin((2 * Math.PI * 2000 * index) / rate) * 8000, true);
  }
  return view.buffer;
}

function desktopApi(): Api {
  return {
    preview: false,
    profile: () => call("profile"),
    createProfile: (name) => call("create_profile", { name }),
    rename: (name) => call("rename", { name }),
    contacts: () => call("contacts"),
    addContact: (card) => call("add_contact", { card }),
    cardSound: () => call("card_sound"),
    listenStart: (rate) => call("listen_start", { rate }),
    // The samples go as the request's body, not as JSON.
    listenHeard: (samples) => call("listen_heard", samples),
    listenStop: () => call("listen_stop"),
    readCard: (card) => call("read_card", { card }),
    acceptContact: (contactId) => call("accept_contact", { contactId }),
    declineContact: (contactId) => call("decline_contact", { contactId }),
    messages: (contactId) => call("messages", { contactId }),
    sendMessage: (contactId, text) => call("send_message", { contactId, text }),
    journey: (messageId) => call("journey", { messageId }),
    markRead: (contactId) => call("mark_read", { contactId }),
    groups: () => call("groups"),
    groupMessages: (groupId) => call("group_messages", { groupId }),
    createGroup: (name, contactIds) => call("create_group", { name, contactIds }),
    addGroupMember: (groupId, contactId) => call("add_group_member", { groupId, contactId }),
    removeGroupMember: (groupId, fingerprint) => call("remove_group_member", { groupId, fingerprint }),
    leaveGroup: (groupId) => call("leave_group", { groupId }),
    sendGroupMessage: (groupId, text) => call("send_group_message", { groupId, text }),
    markGroupRead: (groupId) => call("mark_group_read", { groupId }),
    channels: () => call("channels"),
    joinChannel: (name, password, nick) => call("join_channel", { name, password, nick }),
    leaveChannel: (channelId) => call("leave_channel", { channelId }),
    setChannelNick: (channelId, nick) => call("set_channel_nick", { channelId, nick }),
    posts: (channelId) => call("posts", { channelId }),
    sendPost: (channelId, text) => call("send_post", { channelId, text }),
    shareCard: (channelId) => call("share_card", { channelId }),
    muteAuthor: (channelId, author) => call("mute_author", { channelId, author }),
    unmuteAuthor: (channelId, author) => call("unmute_author", { channelId, author }),
    mutedAuthors: (channelId) => call("muted_authors", { channelId }),
    markChannelRead: (channelId) => call("mark_channel_read", { channelId }),
    sendFile: (target, mime, bytes) => call("send_file", { target, mime, data: toBase64(bytes) }),
    async saveFile(file) {
      const path = await save({ defaultPath: fileName(file) });
      if (!path) return false;
      await call("save_file", { fileId: file.id, path });
      return true;
    },
    async fileUrl(fileId) {
      const cached = fileUrls.get(fileId);
      if (cached) return cached;
      const file = await call<{ mime: string; data: string } | null>("file_data", { fileId });
      if (!file) return null;
      const url = URL.createObjectURL(new Blob([fromBase64(file.data)], { type: file.mime }));
      fileUrls.set(fileId, url);
      return url;
    },
    settings: () => call("settings"),
    setReadReceipts: (on) => call("set_read_receipts", { on }),
    setTor: (on) => call("set_tor", { on }),
    setBluetooth: (on) => call("set_bluetooth", { on }),
    setNearby: (on) => call("set_nearby", { on }),
    nearby: () => call("nearby"),
    wifiAware: () => call("wifi_aware"),
    wifiDirect: () => call("wifi_direct"),
    lora: () => call("lora"),
    setLora: (device) => call("set_lora", { device }),
    loraPorts: () => call("lora_ports"),
    loraScan: () => call("lora_scan"),
    loraFound: () => call("lora_found"),
    reticulum: () => call("reticulum"),
    setReticulum: (address) => call("set_reticulum", { address }),
    restart: () => call("restart"),
    status: () => call("status"),
    async exportFile() {
      const day = new Date().toISOString().slice(0, 10);
      const path = await save({ defaultPath: `carrychat-${day}.carrychat`, filters: FILE_FILTERS });
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
    setNotifications: (on, language) => call("set_notifications", { on, language }),
    onEvent: (handler) => listen<NodeEvent>("carrychat-event", (event) => handler(event.payload)),
    onError: (handler) => listen<string>("carrychat-error", (event) => handler(event.payload)),
  };
}

function previewApi(): Api {
  const now = () => Math.floor(Date.now() / 1000);
  const listeners: ((event: NodeEvent) => void)[] = [];
  const emit = (event: NodeEvent) => listeners.forEach((listener) => listener(event));
  let profile: Profile | null = null;
  let settings: Settings = {
    readReceipts: true,
    tor: false,
    bluetooth: true,
    nearby: false,
    loraDevice: null,
    reticulum: null,
  };
  const minutesAgo = (minutes: number) => Math.floor(Date.now() / 1000) - minutes * 60;
  // Who the preview shows nearby once it is visible.
  const nearbyPeople: NearbyPerson[] = [
    { session: "a1", name: "Elif", fingerprint: "q3rd 7kpa mx2c vuhe 4tzn 6wyb fo", card: "carrychat:elif", link: "bluetooth", contactId: null },
    { session: "b2", name: "Deniz", fingerprint: "7jgi 54zm 2zkd yent vzen s3pg lq", card: "carrychat:deniz", link: "wifiAware", contactId: 1 },
  ];
  let nextId = 10;
  const contact = (id: number, name: string, fingerprint: string, request = false, secure = false): Contact => ({
    id,
    name,
    fingerprint,
    lastMessage: null,
    lastAt: null,
    request,
    unread: 0,
    waiting: 0,
    secure,
    hasOurCard: !request,
  });
  const contacts: Contact[] = [
    contact(1, "Deniz", "7jgi 54zm 2zkd yent vzen s3pg lq", false, true),
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
    file: null,
    via: outgoing ? (done ? ["bluetooth", "lan"] : ["bluetooth"]) : [],
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
  const member = (name: string, fingerprint: string, contactId: number | null, admin = false): Member => ({
    name,
    fingerprint,
    me: false,
    admin,
    contactId,
    card: `carrychat:${fingerprint.replaceAll(" ", "")}`,
  });
  const groups: Group[] = [
    {
      id: 1,
      name: "Mahalle",
      admin: true,
      member: true,
      members: [
        { ...member("Sen", "b3oi uvmj hsk3 wflg ofyx lmwu ci", null, true), me: true },
        member("Deniz", "7jgi 54zm 2zkd yent vzen s3pg lq", 1),
        member("Ayşe", "gp6m vxkt dnvv lqmw ewpx qfox be", 2),
        member("Kerem", "u2ab 5ycq oxwf 7mnt l3dk hz6a qe", null),
      ],
      lastMessage: null,
      lastAt: null,
      unread: 0,
    },
  ];
  const groupMessage = (id: number, sender: string | null, body: string, ago: number): GroupMessage => ({
    id,
    groupId: 1,
    outgoing: sender === null,
    sender,
    body,
    createdAt: now() - ago,
    read: true,
    file: null,
  });
  const groupMessages: GroupMessage[] = [
    groupMessage(101, "Kerem", "Okulun önünde su dağıtımı var, saat 10'da.", 7200),
    groupMessage(102, null, "Teşekkürler, Deniz'le birlikte geliriz.", 7000),
    { ...groupMessage(103, "Ayşe", "Bizim sokakta elektrik geldi.", 900), read: false },
    { ...groupMessage(104, "Kerem", "", 700), file: sampleFile(901, 40, 40) },
    { ...groupMessage(105, "Deniz", "", 120), read: false, file: sampleFile(902, 40, 12) },
  ];
  function sampleFile(id: number, chunks: number, received: number): FileView {
    if (received === chunks) {
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="420"><rect width="640" height="420" fill="#c97c14"/><circle cx="470" cy="120" r="60" fill="#f6e3c4"/><path d="M0 420 L210 190 L330 320 L430 230 L640 420 Z" fill="#1e3440"/></svg>`;
      fileUrls.set(id, URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" })));
    }
    return { id, mime: "image/jpeg", size: 160_000, chunks, received, ready: received === chunks, expiresAt: now() + 86400 };
  }
  const summarizeGroups = () =>
    groups.map((group) => {
      const own = groupMessages.filter((message) => message.groupId === group.id);
      const last = own.at(-1);
      return {
        ...group,
        lastMessage: last?.body ?? null,
        lastAt: last?.createdAt ?? null,
        unread: own.filter((message) => !message.read).length,
      };
    });
  const channels: Channel[] = [
    {
      id: 1,
      name: "kadıköy",
      hasPassword: false,
      nick: "Kemal",
      lastMessage: null,
      lastAt: null,
      unread: 0,
    },
  ];
  const post = (id: number, nick: string | null, author: string, body: string, ago: number): Post => ({
    id,
    channelId: 1,
    outgoing: nick === null,
    nick: nick ?? "Kemal",
    author,
    body,
    sharedCard: null,
    createdAt: now() - ago,
    read: true,
    file: null,
  });
  const posts: Post[] = [
    post(201, "gönüllü", "q4m27hxa", "Moda'da çadır alanı kuruldu, battaniye lazım.", 5400),
    post(202, null, "b3oiuvmj", "Ne kadar lazım? İki tane getirebilirim.", 5000),
    {
      ...post(203, "Selin", "x7tk2pdu", "", 3600),
      sharedCard: { name: "Selin Kaya", fingerprint: "n5df 2kqa 7mvx oe3c wl4t zyrb ka", card: "carrychat:selin" },
    },
    { ...post(204, "gönüllü", "q4m27hxa", "Ne kadar çok o kadar iyi. Teşekkürler!", 600), read: false },
  ];
  const muted: (Muted & { channelId: number })[] = [];
  const summarizeChannels = () =>
    channels.map((channel) => {
      const own = posts.filter((candidate) => candidate.channelId === channel.id);
      const last = own.filter((candidate) => !candidate.sharedCard).at(-1);
      return {
        ...channel,
        lastMessage: last?.body ?? null,
        lastAt: last?.createdAt ?? null,
        unread: own.filter((candidate) => !candidate.read).length,
      };
    });
  const summarize = () =>
    contacts.map((contact) => {
      const own = messages.filter((message) => message.contactId === contact.id);
      const last = own.at(-1);
      return {
        ...contact,
        lastMessage: last?.body ?? null,
        lastAt: last?.createdAt ?? null,
        unread: own.filter((message) => !message.outgoing && !message.read).length,
        waiting: own.filter((message) => message.outgoing && !message.delivered).length,
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
        card: "carrychat:ANBKsjJ0K7SrOhNovUYV5ObQIkq3GgFrr4UgozLJd4c3BSpQdzrI2Rdz8tyWYuEvDe_pFeQVuKHI4gpaPWqyuEMFYWxpY2XdVELCbsBqOz0qO_dKG4ggm5THqxr3I5G2SZI9XsHtAcZk67tdtelzFtx3SYKB-TSNiDYR9ZlQBS4bMi8v4aIF",
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
    async cardSound() {
      return beep();
    },
    async listenStart() {
      listeningSince = Date.now();
    },
    async listenHeard() {
      // The preview hears a card three seconds in.
      return Date.now() - listeningSince > 3000 ? { card: "carrychat:elif", name: "Elif" } : null;
    },
    async listenStop() {},
    async readCard(card) {
      return { card, name: "Elif" };
    },
    async addContact(card) {
      if (!card.trim().startsWith("carrychat:")) throw new Error("invalid card prefix");
      const fromGroup = groups.flatMap((group) => group.members).find((candidate) => candidate.card === card);
      if (fromGroup) {
        const added = contact(nextId++, fromGroup.name, fromGroup.fingerprint);
        contacts.push(added);
        groups.forEach((group) =>
          group.members.forEach((candidate) => {
            if (candidate.card === card) candidate.contactId = added.id;
          }),
        );
        return added;
      }
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
    async journey(messageId) {
      const sent = messages.find((message) => message.id === messageId)!;
      const at = (seconds: number) => sent.createdAt + seconds;
      return {
        createdAt: sent.createdAt,
        deliveredAt: sent.delivered ? at(5800) : null,
        readAt: sent.read ? at(6700) : null,
        steps: sent.via.map((via, index) => ({ via, count: via === "bluetooth" ? 2 : 1, at: at(60 + index * 1900) })),
      };
    },
    async sendMessage(contactId, text) {
      const sent = { ...message(nextId++, contactId, true, text.trim(), 0, false), via: [] as Via[] };
      messages.push(sent);
      setTimeout(() => {
        sent.via = ["lan"];
        emit({ type: "messageMoved", contactId, messageId: sent.id });
      }, 1000);
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
    async groups() {
      return summarizeGroups();
    },
    async groupMessages(groupId) {
      return groupMessages.filter((message) => message.groupId === groupId).map((message) => ({ ...message }));
    },
    async createGroup(name, contactIds) {
      const created: Group = {
        id: nextId++,
        name: name.trim(),
        admin: true,
        member: true,
        members: [
          { ...member(profile?.name ?? "Sen", profile?.fingerprint ?? "", null, true), me: true },
          ...contacts
            .filter((contact) => contactIds.includes(contact.id))
            .map((contact) => member(contact.name, contact.fingerprint, contact.id)),
        ],
        lastMessage: null,
        lastAt: null,
        unread: 0,
      };
      groups.push(created);
      return created;
    },
    async addGroupMember(groupId, contactId) {
      const group = groups.find((candidate) => candidate.id === groupId)!;
      const added = contacts.find((contact) => contact.id === contactId)!;
      group.members.push(member(added.name, added.fingerprint, added.id));
      return { ...group };
    },
    async removeGroupMember(groupId, fingerprint) {
      const group = groups.find((candidate) => candidate.id === groupId)!;
      group.members = group.members.filter((candidate) => candidate.fingerprint !== fingerprint);
      return { ...group };
    },
    async leaveGroup(groupId) {
      const group = groups.find((candidate) => candidate.id === groupId)!;
      group.member = false;
      return { ...group };
    },
    async sendGroupMessage(groupId, text) {
      const sent = { ...groupMessage(nextId++, null, text.trim(), 0), groupId };
      groupMessages.push(sent);
      return { ...sent };
    },
    async markGroupRead(groupId) {
      groupMessages.filter((message) => message.groupId === groupId).forEach((message) => {
        message.read = true;
      });
    },
    async channels() {
      return summarizeChannels();
    },
    async joinChannel(name, password, nick) {
      const normalized = name.trim().replace(/^#/, "").toLowerCase().replace(/i\u0307/g, "i");
      if (!/^[\p{L}\p{N}_-]{1,32}$/u.test(normalized)) throw new Error("invalid channel name");
      const joined = { id: nextId++, name: normalized, hasPassword: password !== "", nick, lastMessage: null, lastAt: null, unread: 0 };
      channels.push(joined);
      return joined;
    },
    async leaveChannel(channelId) {
      channels.splice(channels.findIndex((candidate) => candidate.id === channelId), 1);
    },
    async setChannelNick(channelId, nick) {
      const channel = channels.find((candidate) => candidate.id === channelId)!;
      channel.nick = nick.trim();
      return { ...channel };
    },
    async posts(channelId) {
      const hidden = new Set(muted.filter((entry) => entry.channelId === channelId).map((entry) => entry.author));
      return posts
        .filter((candidate) => candidate.channelId === channelId && !hidden.has(candidate.author))
        .map((candidate) => ({ ...candidate }));
    },
    async sendPost(channelId, text) {
      const channel = channels.find((candidate) => candidate.id === channelId)!;
      const sent = { ...post(nextId++, null, "b3oiuvmj", text.trim(), 0), channelId, nick: channel.nick };
      posts.push(sent);
      return { ...sent };
    },
    async shareCard(channelId) {
      const channel = channels.find((candidate) => candidate.id === channelId)!;
      const sent = {
        ...post(nextId++, null, "b3oiuvmj", "", 0),
        channelId,
        nick: channel.nick,
        sharedCard: { name: profile?.name ?? "", fingerprint: profile?.fingerprint ?? "", card: profile?.card ?? "" },
      };
      posts.push(sent);
      return { ...sent };
    },
    async muteAuthor(channelId, author) {
      const nick = posts.find((candidate) => candidate.author === author)?.nick ?? "";
      muted.push({ channelId, author, nick });
    },
    async unmuteAuthor(channelId, author) {
      const index = muted.findIndex((entry) => entry.channelId === channelId && entry.author === author);
      if (index < 0) throw new Error("that author is not muted here");
      muted.splice(index, 1);
    },
    async mutedAuthors(channelId) {
      return muted.filter((entry) => entry.channelId === channelId).map(({ author, nick }) => ({ author, nick }));
    },
    async sendFile(target, mime, bytes) {
      const id = nextId++;
      const url = URL.createObjectURL(new Blob([bytes.slice()], { type: mime }));
      fileUrls.set(id, url);
      const file: FileView = {
        id,
        mime,
        size: bytes.length,
        chunks: Math.ceil(bytes.length / 4000),
        received: Math.ceil(bytes.length / 4000),
        ready: true,
        expiresAt: now() + 3 * 86400,
      };
      if (target.kind === "contact") {
        const sent = { ...message(nextId++, target.id, true, "", 0, false), file };
        messages.push(sent);
        return sent;
      }
      if (target.kind === "group") {
        const sent = { ...groupMessage(nextId++, null, "", 0), groupId: target.id, file };
        groupMessages.push(sent);
        return sent;
      }
      const channel = channels.find((candidate) => candidate.id === target.id)!;
      const sent = { ...post(nextId++, null, "b3oiuvmj", "", 0), channelId: target.id, nick: channel.nick, file };
      posts.push(sent);
      return sent;
    },
    async fileUrl(fileId) {
      return fileUrls.get(fileId) ?? null;
    },
    async saveFile(file) {
      const url = fileUrls.get(file.id);
      if (!url) return false;
      const link = document.createElement("a");
      link.href = url;
      link.download = fileName(file);
      link.click();
      return true;
    },
    async markChannelRead(channelId) {
      posts.filter((candidate) => candidate.channelId === channelId).forEach((candidate) => {
        candidate.read = true;
      });
    },
    async settings() {
      return settings;
    },
    async setReadReceipts(on) {
      settings = { ...settings, readReceipts: on };
    },
    async setTor(on) {
      settings = { ...settings, tor: on };
    },
    async setBluetooth(on) {
      settings = { ...settings, bluetooth: on };
    },
    async setNearby(on) {
      settings = { ...settings, nearby: on };
      setTimeout(() => emit({ type: "nearbyChanged", people: on ? nearbyPeople.length : 0 }), 600);
    },
    async nearby() {
      return settings.nearby ? nearbyPeople.map((person) => ({ ...person })) : [];
    },
    async wifiAware() {
      return "on";
    },
    async wifiDirect() {
      return "on";
    },
    async lora() {
      const device = settings.loraDevice;
      const meshcore = device !== null && (device.endsWith(":5000") || device.includes("C8:2B:96"));
      return {
        device,
        connected: device !== null,
        firmware: device === null ? null : meshcore ? "MeshCore" : "Meshtastic",
        name: device === null ? null : meshcore ? "Ayşe MeshCore" : "Meshtastic 3f2a",
        region: device === null || meshcore ? null : "EU_868",
        radio: meshcore ? "869.525 MHz, SF11, 250 kHz" : null,
        noFreeChannel: false,
        waiting: 0,
        heard:
          device === null
            ? []
            : meshcore
              ? [{ name: "Kale tekrarlayıcı", at: minutesAgo(95), snr: null, hops: 1 }]
              : [
                  { name: "Tepe röle", at: minutesAgo(4), snr: 6.25, hops: 0 },
                  { name: "Liman", at: minutesAgo(38), snr: -7.5, hops: 2 },
                ],
        heardWithin: meshcore ? 48 * 60 * 60 : 3 * 60 * 60,
        carrychatAt: device === null ? null : minutesAgo(12),
      };
    },
    async setLora(device) {
      settings = { ...settings, loraDevice: device };
      emit({ type: "loraChanged", connected: device !== null });
    },
    async loraPorts() {
      return [{ path: "/dev/cu.usbserial-0001", label: "CP2102 USB to UART Bridge Controller" }];
    },
    async loraScan() {},
    async loraFound() {
      return [
        { address: "D4:8A:FC:3F:2A:01", name: "Meshtastic_3f2a" },
        { address: "C8:2B:96:0E:41:7A", name: "MeshCore-Ayse" },
      ];
    },
    async reticulum() {
      const address = settings.reticulum;
      return {
        address,
        connected: address !== null,
        destination: address === null ? null : "5d2fa0c4e8b17736a9e0c1f42b7d9e13",
        peers:
          address === null
            ? []
            : [
                { destination: "a3c9172e5b0f44d18c26e9b7f0d3a512", hops: 2, at: minutesAgo(7) },
                { destination: "0e7b5c13d9a24f6e81b3c0d7a9e5f264", hops: 4, at: minutesAgo(51) },
              ],
        waiting: 0,
        carrychatAt: address === null ? null : minutesAgo(9),
      };
    },
    async setReticulum(address) {
      settings = { ...settings, reticulum: address };
    },
    async restart() {
      location.reload();
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
        bluetoothOn: settings.bluetooth,
        bluetoothPeers: settings.bluetooth ? 1 : 0,
        bluetoothRadio: true,
        wifiAwarePeers: 1,
        wifiDirectPeers: 0,
        loraConnected: settings.loraDevice !== null,
        loraWaiting: 0,
        internetOn: true,
        internetRelays: 3,
        internetRefusal: null,
        // ?tor=starting, ?tor=ready or ?tor=failed shows the Tor states.
        internetTor: torPreview(),
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
    async setNotifications() {},
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

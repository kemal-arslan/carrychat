import type { Language } from "./i18n";

// Sizes from docs/PROTOCOL.md §5.4, §5.6, §5.7 and §7.
/** A message to a contact, sealed in a ratchet session. */
export const PAIR_OVERHEAD = 104;
/** A group message, with its signature. */
export const GROUP_OVERHEAD = 135;
/** A channel post, with its author key and signature, before the nickname. */
export const CHANNEL_OVERHEAD = 164;
const MAX_BUNDLE = 4096;
const MESHTASTIC_MTU = 233;
const FRAGMENT_HEADER = 11;

export const maxMessageBytes = (overhead: number) => MAX_BUNDLE - overhead;
export const onePacketBytes = (overhead: number) => MESHTASTIC_MTU - 1 - overhead;
export const MAX_NAME_BYTES = 32;

const encoder = new TextEncoder();

export function utf8Length(text: string): number {
  return encoder.encode(text).length;
}

/** How many LoRa packets a message of this many bytes takes. */
export function radioPackets(bytes: number, overhead: number): number {
  const bundle = overhead + bytes;
  if (1 + bundle <= MESHTASTIC_MTU) return 1;
  return Math.ceil(bundle / (MESHTASTIC_MTU - FRAGMENT_HEADER));
}

export function shortTime(unix: number, language: Language): string {
  const date = new Date(unix * 1000);
  const today = date.toDateString() === new Date().toDateString();
  return new Intl.DateTimeFormat(
    language,
    today ? { hour: "2-digit", minute: "2-digit" } : { day: "numeric", month: "short" },
  ).format(date);
}

/** The time of day, for a bubble: the day heading above it gives the date. */
export function clockTime(unix: number, language: Language): string {
  return new Intl.DateTimeFormat(language, { hour: "2-digit", minute: "2-digit" }).format(new Date(unix * 1000));
}

export function fullTime(unix: number, language: Language): string {
  return new Intl.DateTimeFormat(language, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(unix * 1000));
}

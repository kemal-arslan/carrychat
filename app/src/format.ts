import type { Language } from "./i18n";

// Sizes from docs/PROTOCOL.md §5.4 and §7.
const BUNDLE_OVERHEAD = 67;
const MAX_BUNDLE = 4096;
const MESHTASTIC_MTU = 233;
const FRAGMENT_HEADER = 11;

export const MAX_MESSAGE_BYTES = MAX_BUNDLE - BUNDLE_OVERHEAD;
export const ONE_PACKET_BYTES = MESHTASTIC_MTU - 1 - BUNDLE_OVERHEAD;
export const MAX_NAME_BYTES = 32;

const encoder = new TextEncoder();

export function utf8Length(text: string): number {
  return encoder.encode(text).length;
}

/** How many LoRa packets a message of this many bytes takes. */
export function radioPackets(bytes: number): number {
  const bundle = BUNDLE_OVERHEAD + bytes;
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

export function fullTime(unix: number, language: Language): string {
  return new Intl.DateTimeFormat(language, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(unix * 1000));
}

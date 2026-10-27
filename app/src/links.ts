import type { Status, WifiAware } from "./api";
import type { useI18n } from "./i18n";

type Strings = ReturnType<typeof useI18n>["t"];

export type LanState = "off" | "waiting" | "up" | "blocked" | "failed";
export type BluetoothState = "off" | "radioOff" | "waiting" | "up";
export type InternetState = "off" | "waiting" | "up" | "failed";

/** How each link stands, read from the node's status. */
export function linkStates(status: Status | null) {
  const lan: LanState =
    !status || status.lanPort === null
      ? "off"
      : status.lanProblem
        ? status.lanProblem.kind
        : status.lanPeers > 0
          ? "up"
          : "waiting";
  const bluetooth: BluetoothState = !status?.bluetoothOn
    ? "off"
    : status.bluetoothRadio === false
      ? "radioOff"
      : status.bluetoothPeers > 0
        ? "up"
        : "waiting";
  const tor = status?.internetTor ?? null;
  const internet: InternetState = !status?.internetOn
    ? "off"
    : tor?.state === "failed"
      ? "failed"
      : tor?.state !== "starting" && status.internetRelays > 0
        ? "up"
        : "waiting";
  const nearby = (status?.lanPeers ?? 0) + (status?.bluetoothPeers ?? 0) + (status?.wifiAwarePeers ?? 0);
  return { lan, bluetooth, internet, nearby };
}

/** One line for the top of the chat list: whether messages can leave right now. */
export function summary(status: Status | null, t: Strings): { text: string; live: boolean; problem: boolean } {
  const { lan, internet, nearby } = linkStates(status);
  const parts: string[] = [];
  if (internet === "up") parts.push(t.summaryInternet);
  else if (nearby > 0) parts.push(t.summaryNoInternet);
  if (nearby > 0) parts.push(t.summaryNearby(nearby));
  if (lan === "blocked" || lan === "failed") parts.push(t.summaryLanProblem);
  const live = nearby > 0 || internet === "up";
  if (parts.length === 0) parts.push(t.summaryNothing);
  return { text: parts.join(" · "), live, problem: lan === "blocked" || lan === "failed" };
}

export function lanLabel(state: LanState, status: Status | null, t: Strings): string {
  return {
    off: t.stateOff,
    waiting: t.stateLooking,
    up: t.nearbyCount(status?.lanPeers ?? 0),
    blocked: t.stateBlocked,
    failed: t.stateFailed,
  }[state];
}

export function bluetoothLabel(state: BluetoothState, status: Status | null, t: Strings): string {
  return {
    off: t.stateOff,
    radioOff: t.bluetoothRadioOff,
    waiting: t.stateLooking,
    up: t.nearbyCount(status?.bluetoothPeers ?? 0),
  }[state];
}

export function wifiAwareLabel(state: WifiAware, status: Status | null, t: Strings): string {
  if (state === "unsupported") return t.wifiAwareUnsupported;
  if (state === "off") return t.wifiAwareOff;
  const peers = status?.wifiAwarePeers ?? 0;
  return peers > 0 ? t.nearbyCount(peers) : t.stateLooking;
}

export function internetLabel(state: InternetState, status: Status | null, t: Strings): string {
  const tor = status?.internetTor ?? null;
  if (state === "off") return t.stateOff;
  if (state === "failed") return t.stateTorFailed;
  if (state === "up") return t.stateRelays(status?.internetRelays ?? 0);
  return tor?.state === "starting" ? t.stateTorStarting : t.stateConnecting;
}

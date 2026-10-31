// The app lock (§10). With it on, the app asks for the device's owner, by fingerprint, face, or the device's own
// PIN, pattern or password, when it opens and when it comes back after a while out of sight. While it is locked
// nothing of it shows, and nothing counts as read.

import { useEffect, useState } from "react";
import { api } from "./api";
import { appLockOn, setAppLockOn } from "./prefs";

/** How long the app may be out of sight before it locks again: long enough to pick a photo or answer a call. */
const LOCK_AFTER_MS = 60_000;

let locked = appLockOn();
/** The owner is being asked for, which may take the app out of sight for a moment. */
let asking = false;
let hiddenAt: number | null = null;
const watchers = new Set<(locked: boolean) => void>();

function setLocked(next: boolean) {
  locked = next;
  watchers.forEach((watch) => watch(next));
}

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden") {
    hiddenAt ??= Date.now();
    return;
  }
  const away = hiddenAt === null ? 0 : Date.now() - hiddenAt;
  hiddenAt = null;
  if (appLockOn() && !asking && away >= LOCK_AFTER_MS) setLocked(true);
});

export function useAppLocked(): boolean {
  const [state, setState] = useState(locked);
  useEffect(() => {
    watchers.add(setState);
    setState(locked);
    return () => {
      watchers.delete(setState);
    };
  }, []);
  return state;
}

/** Whether someone is looking at the app: it has the focus and is not locked. */
export function looking(): boolean {
  return document.hasFocus() && !locked;
}

/** Whether this device can ask for its owner: it has a fingerprint, face, or a screen lock or password. */
export async function lockAvailable(): Promise<boolean> {
  const android = window.CarryChatAndroid;
  if (android) return android.lockAvailable?.() ?? false;
  return api.lockAvailable().catch(() => false);
}

/** Asks for the device's owner. Resolves once they are there; fails, with the system's words, if not. */
function askForOwner(title: string, reason: string): Promise<void> {
  asking = true;
  const android = window.CarryChatAndroid;
  const asked = android
    ? new Promise<void>((resolve, reject) => {
        const answer = (event: Event) => {
          window.removeEventListener("carrychat-owner", answer);
          const { ok, error } = (event as CustomEvent<{ ok: boolean; error: string | null }>).detail;
          if (ok) resolve();
          else reject(new Error(error ?? ""));
        };
        window.addEventListener("carrychat-owner", answer);
        android.authenticate(title, reason);
      })
    : api.authenticate(reason);
  return asked.finally(() => {
    asking = false;
  });
}

/** Opens the locked app once its owner is there. */
export async function unlock(title: string, reason: string): Promise<void> {
  await askForOwner(title, reason);
  setLocked(false);
}

/** Turns the app lock on or off, asking for the owner first: nobody else can turn it off, nor on unknowingly. */
export async function setAppLock(on: boolean, title: string, reason: string): Promise<void> {
  await askForOwner(title, reason);
  setAppLockOn(on);
  window.CarryChatAndroid?.setAppLock(on);
}

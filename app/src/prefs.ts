// Preferences that belong to this window rather than to the node.

const NOTIFICATIONS_KEY = "carrychat.notifications";
const INAUDIBLE_KEY = "carrychat.soundInaudible";
const APP_LOCK_KEY = "carrychat.appLock";

export function notificationsEnabled(): boolean {
  try {
    return localStorage.getItem(NOTIFICATIONS_KEY) !== "off";
  } catch {
    return true;
  }
}

export function setNotificationsEnabled(on: boolean): void {
  try {
    if (on) localStorage.removeItem(NOTIFICATIONS_KEY);
    else localStorage.setItem(NOTIFICATIONS_KEY, "off");
  } catch {
    // Without storage the setting lasts until the window closes.
  }
}

/**
 * Whether cards and messages play as sound above hearing, at 18 to 20 kHz: so they do, unless turned off for a
 * speaker that cannot play that high.
 */
export function soundInaudible(): boolean {
  try {
    return localStorage.getItem(INAUDIBLE_KEY) !== "off";
  } catch {
    return true;
  }
}

export function setSoundInaudible(on: boolean): void {
  try {
    if (on) localStorage.removeItem(INAUDIBLE_KEY);
    else localStorage.setItem(INAUDIBLE_KEY, "off");
  } catch {
    // Without storage the setting lasts until the window closes.
  }
}

/** Whether the app asks for the device's owner when it opens and comes back (§10). */
export function appLockOn(): boolean {
  try {
    return localStorage.getItem(APP_LOCK_KEY) === "on";
  } catch {
    return false;
  }
}

export function setAppLockOn(on: boolean): void {
  try {
    if (on) localStorage.setItem(APP_LOCK_KEY, "on");
    else localStorage.removeItem(APP_LOCK_KEY);
  } catch {
    // Without storage the setting lasts until the window closes.
  }
}

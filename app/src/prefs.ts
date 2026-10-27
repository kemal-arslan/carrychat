// Preferences that belong to this window rather than to the node.

const NOTIFICATIONS_KEY = "carrychat.notifications";
const INAUDIBLE_KEY = "carrychat.soundInaudible";

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

/** Whether cards and messages play as sound above hearing, at 18 to 20 kHz. */
export function soundInaudible(): boolean {
  try {
    return localStorage.getItem(INAUDIBLE_KEY) === "on";
  } catch {
    return false;
  }
}

export function setSoundInaudible(on: boolean): void {
  try {
    if (on) localStorage.setItem(INAUDIBLE_KEY, "on");
    else localStorage.removeItem(INAUDIBLE_KEY);
  } catch {
    // Without storage the setting lasts until the window closes.
  }
}

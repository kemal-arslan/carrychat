// Preferences that belong to this window rather than to the node.

const NOTIFICATIONS_KEY = "kervan.notifications";

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

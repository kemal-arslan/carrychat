// Preferences that belong to this window rather than to the node.

const NOTIFICATIONS_KEY = "carrychat.notifications";
const SOUND_BAND_KEY = "carrychat.soundBand";
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
 * Where cards and messages play as sound: `far`, one tone at a time, reaching furthest but slowly; `farInaudible`,
 * the same at 18 to 19.5 kHz, unheard; `audible`, fast, for a device side by side; `inaudible`, fast at 18 to
 * 20 kHz, side by side and unheard. Far unless chosen otherwise.
 */
export type SoundBand = "far" | "farInaudible" | "audible" | "inaudible";

const SOUND_BANDS: SoundBand[] = ["far", "farInaudible", "audible", "inaudible"];

export function soundBand(): SoundBand {
  try {
    const stored = localStorage.getItem(SOUND_BAND_KEY);
    return SOUND_BANDS.find((band) => band === stored) ?? "far";
  } catch {
    return "far";
  }
}

/** Whether the band chosen plays above hearing, so nothing is heard while it plays. */
export function soundUnheard(): boolean {
  const band = soundBand();
  return band === "inaudible" || band === "farInaudible";
}

export function setSoundBand(band: SoundBand): void {
  try {
    if (band === "far") localStorage.removeItem(SOUND_BAND_KEY);
    else localStorage.setItem(SOUND_BAND_KEY, band);
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

// Light or dark: the system's, unless the user picks one under Me.

export type ThemeChoice = "system" | "light" | "dark";

const STORAGE_KEY = "carrychat.theme";
const systemDark = window.matchMedia("(prefers-color-scheme: dark)");

declare global {
  interface Window {
    /** On Android, the activity colours the status and navigation bars. */
    CarryChatAndroid?: { setDark(dark: boolean): void };
  }
}

export function themeChoice(): ThemeChoice {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return stored === "light" || stored === "dark" ? stored : "system";
  } catch {
    return "system";
  }
}

export function setThemeChoice(choice: ThemeChoice): void {
  try {
    if (choice === "system") localStorage.removeItem(STORAGE_KEY);
    else localStorage.setItem(STORAGE_KEY, choice);
  } catch {
    // Without storage the choice lasts until the window closes.
  }
  applyTheme();
}

/** Sets the page's theme, and the phone's bars to match. */
export function applyTheme(): void {
  const choice = themeChoice();
  const dark = choice === "dark" || (choice === "system" && systemDark.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  window.CarryChatAndroid?.setDark(dark);
}

systemDark.addEventListener("change", applyTheme);

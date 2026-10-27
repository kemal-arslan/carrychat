// The Android back button, taken over from the web view, whose history it
// cannot rely on: the web view skips entries a page added on its own.

import { onBackButtonPress } from "@tauri-apps/api/app";
import { useEffect, useRef } from "react";

/** What the back button would undo, newest last. */
const steps: (() => void)[] = [];

/**
 * While `active`, lets the back button take a step back, such as closing a
 * sheet or leaving a conversation. The newest step goes first.
 */
export function useBackStep(active: boolean, step: () => void) {
  const latest = useRef(step);
  latest.current = step;
  useEffect(() => {
    if (!active) return;
    const entry = () => latest.current();
    steps.push(entry);
    return () => {
      const index = steps.indexOf(entry);
      if (index >= 0) steps.splice(index, 1);
    };
  }, [active]);
}

/** On Android, takes the newest step back, or with none left sends the app behind other apps, as Home does. */
export function handleBackButton() {
  if (!window.CarryChatAndroid) return;
  onBackButtonPress(() => {
    const step = steps[steps.length - 1];
    if (step) step();
    else window.CarryChatAndroid?.leave();
  });
}

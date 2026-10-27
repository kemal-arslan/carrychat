// Cards over sound (PROTOCOL.md §3.2): playing this device's card, and
// listening for another's. The node makes and reads the sound; the page only
// plays it and hands over what the microphone hears.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, type HeardCard } from "./api";

/** How often the microphone's samples go to the listener. */
const SEND_MS = 250;

export function canListen(): boolean {
  return Boolean(navigator.mediaDevices?.getUserMedia) && typeof AudioContext !== "undefined";
}

/** Plays this device's card over and over. Returns what stops it. */
export async function playCard(): Promise<() => void> {
  const wav = await api.cardSound();
  const context = new AudioContext();
  const source = context.createBufferSource();
  source.buffer = await context.decodeAudioData(wav);
  source.loop = true;
  source.connect(context.destination);
  source.start();
  let stopped = false;
  return () => {
    if (stopped) return;
    stopped = true;
    source.stop();
    void context.close();
  };
}

/**
 * Listens to the microphone until a card is heard, and gives it to `onHeard`. Returns what stops listening,
 * which also stops by itself once a card is heard.
 */
export async function listenForCard(onHeard: (card: HeardCard) => void): Promise<() => void> {
  // The browser's voice processing would take the tones for noise.
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false },
  });
  const context = new AudioContext();
  await api.listenStart(context.sampleRate);
  const input = context.createMediaStreamSource(stream);
  // ScriptProcessor runs in every web view the app does; what it outputs stays silent.
  const processor = context.createScriptProcessor(4096, 1, 1);
  let heard: Int16Array[] = [];
  processor.onaudioprocess = (event) => {
    const samples = event.inputBuffer.getChannelData(0);
    const chunk = new Int16Array(samples.length);
    for (let index = 0; index < samples.length; index++) {
      chunk[index] = Math.max(-1, Math.min(1, samples[index])) * 32767;
    }
    heard.push(chunk);
  };
  input.connect(processor);
  processor.connect(context.destination);

  let stopped = false;
  let sending = false;
  const stop = () => {
    if (stopped) return;
    stopped = true;
    window.clearInterval(timer);
    processor.disconnect();
    input.disconnect();
    stream.getTracks().forEach((track) => track.stop());
    void context.close();
    void api.listenStop();
  };
  const timer = window.setInterval(async () => {
    if (sending || heard.length === 0) return;
    sending = true;
    const samples = new Int16Array(heard.reduce((sum, chunk) => sum + chunk.length, 0));
    let at = 0;
    for (const chunk of heard) {
      samples.set(chunk, at);
      at += chunk.length;
    }
    heard = [];
    try {
      const card = await api.listenHeard(new Uint8Array(samples.buffer));
      if (card && !stopped) {
        stop();
        onHeard(card);
      }
    } finally {
      sending = false;
    }
  }, SEND_MS);
  return stop;
}

/**
 * Plays or listens for a sheet, one thing at a time. `start` shows it as on at once, while the microphone may still
 * be asking for permission; `stop`, or closing the sheet, ends it, even one that has not started yet.
 */
export function useSound() {
  const [active, setActive] = useState(false);
  const run = useRef(0);
  const end = useRef<(() => void) | null>(null);

  const stop = useCallback(() => {
    run.current += 1;
    end.current?.();
    end.current = null;
    setActive(false);
  }, []);

  useEffect(() => stop, [stop]);

  async function start(begin: () => Promise<() => void>) {
    stop();
    const mine = run.current;
    setActive(true);
    try {
      const stopIt = await begin();
      if (run.current === mine) end.current = stopIt;
      else stopIt();
    } catch (error) {
      if (run.current === mine) setActive(false);
      throw error;
    }
  }

  return { active, start, stop };
}

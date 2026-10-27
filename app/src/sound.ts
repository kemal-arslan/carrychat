// Sound (PROTOCOL.md §3.2, §9.10): playing this device's card or one of its
// messages, and listening for another's card or messages. The node makes
// and reads the sound; the page only plays it and hands over what the
// microphone hears.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, type HeardCard } from "./api";
import { soundInaudible } from "./prefs";

/** How often the microphone's samples go to the listener. */
const SEND_MS = 250;
/** How many times a message is played, for a listener that misses a round. */
const MESSAGE_PLAYS = 3;

export function canListen(): boolean {
  return Boolean(navigator.mediaDevices?.getUserMedia) && typeof AudioContext !== "undefined";
}

/** Plays this device's card over and over, above hearing if so set. Returns what stops it. */
export async function playCard(): Promise<() => void> {
  const wav = await api.cardSound(soundInaudible());
  const context = new AudioContext();
  const source = context.createBufferSource();
  try {
    source.buffer = await context.decodeAudioData(wav);
    source.loop = true;
    source.connect(context.destination);
    source.start();
  } catch (error) {
    void context.close();
    throw error;
  }
  let stopped = false;
  return () => {
    if (stopped) return;
    stopped = true;
    source.stop();
    void context.close();
  };
}

/** Messages played one after another, so two sent quickly do not sound over each other. */
let messagesPlaying: Promise<void> = Promise.resolve();

/** Plays one of our messages as sound a few times, above hearing if so set; resolves once it is done. */
export function playMessage(messageId: number): Promise<void> {
  const played = messagesPlaying.then(async () => {
    const wav = await api.messageSound(messageId, soundInaudible());
    const context = new AudioContext();
    try {
      const source = context.createBufferSource();
      source.buffer = await context.decodeAudioData(wav);
      source.loop = true;
      source.connect(context.destination);
      const ended = new Promise((resolve) => (source.onended = resolve));
      source.start();
      source.stop(context.currentTime + source.buffer.duration * MESSAGE_PLAYS);
      await ended;
    } finally {
      void context.close();
    }
  });
  messagesPlaying = played.catch(() => {});
  return played;
}

/** Whether the page listens for messages played as sound: off, on, or on but without a microphone. */
export type MessageListening = "off" | "on" | "failed";

let wantMessages = false;
let stopMessages: (() => void) | null = null;
/** Message listening is starting: the microphone is opening. */
let startingMessages = false;
let cardListenings = 0;
let messageListening: MessageListening = "off";
const watchers = new Set<(state: MessageListening) => void>();

function setMessageListening(state: MessageListening) {
  messageListening = state;
  watchers.forEach((watch) => watch(state));
}

async function startMessages() {
  if (startingMessages || stopMessages || cardListenings > 0 || !wantMessages) return;
  startingMessages = true;
  try {
    const stop = await listen(null);
    // Turned off, or a card listening began, while the microphone opened.
    if (!wantMessages || cardListenings > 0) {
      stop();
      return;
    }
    stopMessages = stop;
  } catch {
    // Not asked again until it is turned on again.
    wantMessages = false;
    setMessageListening("failed");
  } finally {
    startingMessages = false;
  }
}

/**
 * Listens for messages played as sound while `on`, and while the app is open: the node takes in what it hears like
 * a message from any link. Listening for a card pauses it.
 */
export function listenForMessages(on: boolean) {
  wantMessages = on;
  if (on) {
    // On at once, while the microphone may still be asking for permission.
    setMessageListening("on");
    void startMessages();
  } else {
    stopMessages?.();
    stopMessages = null;
    setMessageListening("off");
  }
}

export function useMessageListening(): MessageListening {
  const [state, setState] = useState(messageListening);
  useEffect(() => {
    watchers.add(setState);
    return () => {
      watchers.delete(setState);
    };
  }, []);
  return state;
}

/**
 * Listens to the microphone until a card is heard, and gives it to `onHeard`. Returns what stops listening,
 * which also stops by itself once a card is heard. Listening for messages pauses meanwhile.
 */
export async function listenForCard(onHeard: (card: HeardCard) => void): Promise<() => void> {
  cardListenings += 1;
  stopMessages?.();
  stopMessages = null;
  let ended = false;
  const end = () => {
    if (ended) return;
    ended = true;
    cardListenings -= 1;
    void startMessages();
  };
  try {
    const stop = await listen((card) => {
      end();
      onHeard(card);
    });
    return () => {
      stop();
      end();
    };
  } catch (error) {
    end();
    throw error;
  }
}

/**
 * Listens to the microphone, handing what it hears to the node: messages heard go to it, and a card heard goes to
 * `onCard`, if given, which ends listening. Returns what stops listening.
 */
async function listen(onCard: ((card: HeardCard) => void) | null): Promise<() => void> {
  // The browser's voice processing would take the tones for noise.
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false },
  });
  let context: AudioContext | null = null;
  let listening: number;
  try {
    context = new AudioContext();
    listening = await api.listenStart(context.sampleRate);
  } catch (error) {
    // The microphone goes off again if listening could not start.
    stream.getTracks().forEach((track) => track.stop());
    void context?.close();
    throw error;
  }
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
    void api.listenStop(listening);
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
      const card = await api.listenHeard(listening, samples);
      if (card && onCard && !stopped) {
        stop();
        onCard(card);
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

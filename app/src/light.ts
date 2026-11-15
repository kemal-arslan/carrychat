// Light (PROTOCOL.md §9.11): sending one of our messages by blinking the
// flashlight, and watching another phone's flashlight through the camera.
// The node makes the light and reads it; the page only has the Android
// activity blink it, and hands over how bright the camera sees it.

import { api, type LightWatch } from "./api";

/** How often the camera's pictures go to the watcher. */
const SEND_MS = 250;
/** The box watched, as a share of the picture's shorter side. */
export const BOX = 0.2;
/** The pixels in the box that blink most, whose brightness is the light's. */
const SPOT_PIXELS = 24;
/** How fast each pixel's mean and spread follow the picture: about a second at 30 pictures a second. */
const FOLLOW = 0.04;
/** Pictures between choosing the pixels that blink most again. */
const CHOOSE_EVERY = 10;

/** Where the preview pretends to have started blinking, for the page in a plain browser. */
let previewSince: number | null = null;

/** Whether this device can send by light: an Android phone with a flashlight, or the preview. */
export function canSendLight(): boolean {
  const android = window.CarryChatAndroid;
  if (android) return android.lightAvailable?.() ?? false;
  return api.preview;
}

/**
 * Whether this device can watch for light: Android, or the preview. iOS and macOS would first need the camera's
 * usage description, and on macOS its entitlement, or the system stops the app when the camera opens.
 */
export function canWatchLight(): boolean {
  return Boolean(navigator.mediaDevices?.getUserMedia) && (Boolean(window.CarryChatAndroid) || api.preview);
}

export function keepScreenOn(on: boolean) {
  window.CarryChatAndroid?.keepScreenOn?.(on);
}

/** Starts blinking one of our messages over and over; gives how long one pass takes, in milliseconds. */
export async function sendByLight(messageId: number): Promise<number> {
  const { pattern, symbolMs } = await api.messageLight(messageId);
  const android = window.CarryChatAndroid;
  if (android) {
    if (!android.lightPlay?.(pattern, symbolMs)) throw new Error("no flashlight");
  } else {
    previewSince = Date.now();
  }
  return pattern.length * symbolMs;
}

export function stopSendingLight() {
  window.CarryChatAndroid?.lightStop?.();
  previewSince = null;
}

/** Whether the flashlight still blinks: it stops by itself if the camera is taken. */
export function sendingLight(): boolean {
  const android = window.CarryChatAndroid;
  return android ? (android.lightPlaying?.() ?? false) : previewSince !== null;
}

/** The exposure the camera keeps while watching, in its units of 100 µs: 5 ms. */
const EXPOSURE = 50;
/** The sensitivity it keeps meanwhile, so a light far away still shows. */
const ISO = 800;

/**
 * Has the camera keep a short exposure of its own, where it lets the page set one. Left to itself, a camera in the
 * dark lengthens its exposure to 30 ms and takes 20 pictures a second, too few for the light's 50 ms bits; held at
 * 5 ms, the phones measured took 36 to 47, and a flashlight stood far above the dark around it.
 */
async function fastPictures(track: MediaStreamTrack | undefined) {
  if (!track?.getCapabilities) return;
  const capabilities = track.getCapabilities() as {
    exposureMode?: string[];
    exposureTime?: { min: number; max: number };
    iso?: { min: number; max: number };
  };
  if (!capabilities.exposureMode?.includes("manual") || !capabilities.exposureTime) return;
  const exposure = Math.min(Math.max(EXPOSURE, capabilities.exposureTime.min), capabilities.exposureTime.max);
  const iso = capabilities.iso ? Math.min(Math.max(ISO, capabilities.iso.min), capabilities.iso.max) : undefined;
  const manual = { exposureMode: "manual", exposureTime: exposure, ...(iso ? { iso } : {}) };
  await track
    .applyConstraints({ frameRate: { ideal: 60 }, advanced: [manual as MediaTrackConstraintSet] })
    .catch(() => {});
}

/** The camera's zoom, where it has one. */
export type Zoom = { min: number; max: number; step: number; value: number; set: (value: number) => void };

/** Where the box is: its centre, as shares of the picture's width and height. */
export type Aim = { x: number; y: number };

/**
 * Finds the light in the box: it follows how much each pixel's brightness
 * changes, and takes the brightness of those that change most, so a street
 * light or a lit window beside the flashlight does not count.
 */
class Spot {
  private size = 0;
  private pictures = 0;
  private luma = new Float32Array(0);
  private mean = new Float32Array(0);
  private spread = new Float32Array(0);
  private chosen = new Uint32Array(0);

  reset() {
    this.size = 0;
  }

  brightness(pixels: Uint8ClampedArray, size: number): number {
    const count = size * size;
    if (this.size !== size) {
      this.size = size;
      this.pictures = 0;
      this.luma = new Float32Array(count);
      this.mean = new Float32Array(count);
      this.spread = new Float32Array(count);
      this.chosen = new Uint32Array(0);
    }
    const { luma, mean, spread } = this;
    for (let index = 0; index < count; index++) {
      const at = index * 4;
      const value = (pixels[at] * 77 + pixels[at + 1] * 150 + pixels[at + 2] * 29) / 256;
      luma[index] = value;
      if (this.pictures === 0) {
        mean[index] = value;
        continue;
      }
      const away = value - mean[index];
      mean[index] += FOLLOW * away;
      spread[index] += FOLLOW * (away * away - spread[index]);
    }
    if (this.pictures % CHOOSE_EVERY === 0) {
      const order = Array.from({ length: count }, (_, index) => index);
      order.sort((a, b) => spread[b] - spread[a]);
      this.chosen = Uint32Array.from(order.slice(0, SPOT_PIXELS));
    }
    this.pictures += 1;
    let sum = 0;
    for (const index of this.chosen) sum += luma[index];
    return sum / this.chosen.length;
  }
}

/**
 * Shows the back camera in `video` and watches the box at `aim()` for another phone's flashlight, giving each step
 * of the watch to `onWatch`. Gives what stops watching, and the camera's zoom if it has one.
 */
export async function watchLight(
  video: HTMLVideoElement,
  aim: () => Aim,
  onWatch: (watch: LightWatch) => void,
): Promise<{ stop: () => void; zoom: Zoom | null }> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: false,
    video: { facingMode: { ideal: "environment" }, frameRate: { ideal: 60 }, width: { ideal: 1280 }, height: { ideal: 720 } },
  });
  await fastPictures(stream.getVideoTracks()[0]);
  let watching: number;
  try {
    watching = await api.lightStart();
  } catch (error) {
    stream.getTracks().forEach((track) => track.stop());
    throw error;
  }
  video.muted = true;
  video.playsInline = true;
  video.srcObject = stream;
  await video.play().catch(() => {});

  const canvas = document.createElement("canvas");
  const context = canvas.getContext("2d", { willReadFrequently: true });
  const spot = new Spot();
  let pictures: [number, number][] = [];
  let corner = "";
  let stopped = false;
  let sending = false;
  // When each picture was taken, as one clock: the camera's, if it says.
  let clock: "capture" | "display" | null = null;

  const look = (when: number) => {
    const width = video.videoWidth;
    const height = video.videoHeight;
    if (!context || !width || !height) return;
    const side = Math.max(8, Math.round(Math.min(width, height) * BOX));
    const { x, y } = aim();
    const left = Math.min(Math.max(0, Math.round(x * width - side / 2)), width - side);
    const top = Math.min(Math.max(0, Math.round(y * height - side / 2)), height - side);
    // A box moved watches other pixels, so it starts afresh.
    if (`${left},${top}` !== corner) {
      corner = `${left},${top}`;
      spot.reset();
    }
    if (canvas.width !== side) {
      canvas.width = side;
      canvas.height = side;
    }
    context.drawImage(video, left, top, side, side, 0, 0, side, side);
    pictures.push([when, spot.brightness(context.getImageData(0, 0, side, side).data, side)]);
  };

  // Each picture once, with when it was taken; animation frames where the web view lacks that.
  const perPicture = "requestVideoFrameCallback" in HTMLVideoElement.prototype;
  const next = () => {
    if (stopped) return;
    if (perPicture) video.requestVideoFrameCallback(onPicture);
    else requestAnimationFrame((now) => onPicture(now));
  };
  const onPicture = (now: number, metadata?: VideoFrameCallbackMetadata) => {
    if (stopped) return;
    clock ??= metadata?.captureTime !== undefined ? "capture" : "display";
    const when = clock === "capture" ? metadata?.captureTime : (metadata?.expectedDisplayTime ?? now);
    look(when ?? now);
    next();
  };
  next();

  const timer = window.setInterval(async () => {
    if (sending || pictures.length === 0) return;
    sending = true;
    const batch = pictures;
    pictures = [];
    try {
      const watch = await api.lightSeen(watching, batch);
      if (watch && !stopped) onWatch(watch);
    } catch {
      // The next pictures go anyway.
    } finally {
      sending = false;
    }
  }, SEND_MS);

  const [track] = stream.getVideoTracks();
  const capabilities = (track?.getCapabilities?.() ?? {}) as { zoom?: { min: number; max: number; step?: number } };
  const settings = (track?.getSettings?.() ?? {}) as { zoom?: number };
  const zoom: Zoom | null =
    track && capabilities.zoom && capabilities.zoom.max > capabilities.zoom.min
      ? {
          min: capabilities.zoom.min,
          max: capabilities.zoom.max,
          step: capabilities.zoom.step || 0.1,
          value: settings.zoom ?? capabilities.zoom.min,
          set: (value) => {
            spot.reset();
            void track.applyConstraints({ advanced: [{ zoom: value } as MediaTrackConstraintSet] }).catch(() => {});
          },
        }
      : null;

  const stop = () => {
    if (stopped) return;
    stopped = true;
    window.clearInterval(timer);
    stream.getTracks().forEach((track) => track.stop());
    video.srcObject = null;
    void api.lightStop(watching);
  };
  return { stop, zoom };
}

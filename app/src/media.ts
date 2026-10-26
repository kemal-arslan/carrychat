// Getting photos and voice messages small enough to send (PROTOCOL.md §5.9).

/** The largest file a message can offer. */
export const MAX_FILE_BYTES = 1 << 20;
const MAX_SIDE = 1280;
/** Voice messages stop by themselves after this long. */
export const MAX_RECORDING_SECS = 120;

/** A photo as a JPEG of at most 1280 pixels a side and 1 MiB. */
export async function shrinkPhoto(file: File): Promise<Uint8Array> {
  const bitmap = await createImageBitmap(file);
  const scale = Math.min(1, MAX_SIDE / Math.max(bitmap.width, bitmap.height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(bitmap.width * scale);
  canvas.height = Math.round(bitmap.height * scale);
  canvas.getContext("2d")!.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  for (const quality of [0.82, 0.7, 0.55, 0.4]) {
    const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/jpeg", quality));
    if (blob && blob.size <= MAX_FILE_BYTES) return new Uint8Array(await blob.arrayBuffer());
  }
  throw new Error("photo too large");
}

export function canRecord(): boolean {
  return typeof MediaRecorder !== "undefined" && Boolean(navigator.mediaDevices?.getUserMedia);
}

export type Recorder = {
  /** Ends the recording and gives the audio. */
  stop: () => Promise<{ mime: string; bytes: Uint8Array }>;
  /** Ends the recording and throws the audio away. */
  cancel: () => void;
};

/** Starts recording from the microphone. Either way it ends, the microphone turns off. */
export async function startRecording(): Promise<Recorder> {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  const mimeType = ["audio/mp4", "audio/webm;codecs=opus", "audio/webm"].find((type) =>
    MediaRecorder.isTypeSupported(type),
  );
  const recorder = new MediaRecorder(stream, { mimeType, audioBitsPerSecond: 32_000 });
  const parts: Blob[] = [];
  recorder.ondataavailable = (event) => parts.push(event.data);
  recorder.start();
  const release = () => stream.getTracks().forEach((track) => track.stop());
  return {
    cancel: () => {
      recorder.onstop = null;
      if (recorder.state !== "inactive") recorder.stop();
      release();
    },
    stop: () =>
      new Promise((resolve) => {
        recorder.onstop = async () => {
          release();
          const mime = (recorder.mimeType || "audio/mp4").split(";")[0];
          const blob = new Blob(parts, { type: mime });
          resolve({ mime, bytes: new Uint8Array(await blob.arrayBuffer()) });
        };
        recorder.stop();
      }),
  };
}

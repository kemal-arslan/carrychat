import { useEffect, useRef, useState, type ChangeEvent, type KeyboardEvent } from "react";
import { maxMessageBytes, onePacketBytes, radioPackets, utf8Length } from "./format";
import { useI18n } from "./i18n";
import { MAX_RECORDING_SECS, canRecord, shrinkPhoto, startRecording, type Recorder } from "./media";

type Props = {
  placeholder: string;
  /** Bytes a bundle adds around the text, which sets the radio packet count. */
  overhead: number;
  /** Clears the draft when it changes: the conversation it belongs to. */
  conversation: string;
  onSend: (text: string) => Promise<void>;
  /** Sends a photo or voice message. Without it there are no buttons for them. */
  onSendFile?: (mime: string, bytes: Uint8Array) => Promise<void>;
};

type Recording = Recorder & { started: number };

/** A touch keyboard's Enter starts a new line; the Send button sends. */
const TOUCH = window.matchMedia("(pointer: coarse)").matches;

export function Composer({ placeholder, overhead, conversation, onSend, onSendFile }: Props) {
  const { t } = useI18n();
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [recording, setRecording] = useState<Recording | null>(null);
  const [seconds, setSeconds] = useState(0);
  const photoInput = useRef<HTMLInputElement>(null);
  const recordingRef = useRef<Recording | null>(null);
  recordingRef.current = recording;

  // A recording belongs to the conversation it started in: moving to another
  // one, or leaving, throws it away and turns the microphone off, so it can
  // never go to someone else.
  useEffect(() => {
    setDraft("");
    setError(null);
    return () => {
      recordingRef.current?.cancel();
      recordingRef.current = null;
      setRecording(null);
    };
  }, [conversation]);

  // The timer while recording, which also stops a recording that runs too long.
  useEffect(() => {
    if (!recording) return;
    const timer = window.setInterval(() => {
      const elapsed = Math.floor((Date.now() - recording.started) / 1000);
      setSeconds(elapsed);
      if (elapsed >= MAX_RECORDING_SECS) stopRecording();
    }, 250);
    return () => window.clearInterval(timer);
  }, [recording]);

  async function sendFile(work: () => Promise<{ mime: string; bytes: Uint8Array }>) {
    if (!onSendFile) return;
    setSending(true);
    setError(null);
    try {
      const { mime, bytes } = await work();
      await onSendFile(mime, bytes);
    } catch (failure) {
      const text = String(failure).replace(/^Error: /, "");
      setError(text.includes("too large") || text.includes("file size") ? t.fileTooBig : text);
    } finally {
      setSending(false);
    }
  }

  function pickPhoto(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (file) sendFile(async () => ({ mime: "image/jpeg", bytes: await shrinkPhoto(file) }));
  }

  async function record() {
    setError(null);
    try {
      const started = await startRecording();
      setSeconds(0);
      setRecording({ ...started, started: Date.now() });
    } catch {
      setError(t.micDenied);
    }
  }

  function stopRecording() {
    if (!recording) return;
    const current = recording;
    setRecording(null);
    sendFile(() => current.stop());
  }

  const bytes = utf8Length(draft.trim());
  const max = maxMessageBytes(overhead);
  const tooLong = bytes > max;
  const packets = radioPackets(bytes, overhead);

  async function send() {
    if (bytes === 0 || tooLong || sending) return;
    setSending(true);
    setError(null);
    try {
      await onSend(draft);
      setDraft("");
    } catch (failure) {
      setError(String(failure).replace(/^Error: /, ""));
    } finally {
      setSending(false);
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (TOUCH) return;
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      send();
    }
  }

  return (
    <div className="composer">
      <textarea
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={onKeyDown}
        placeholder={placeholder}
        rows={2}
        aria-label={placeholder}
      />
      <div className="composer-row">
        <span className={tooLong ? "meter error" : bytes > onePacketBytes(overhead) ? "meter warn" : "meter"}>
          {error ??
            (recording
              ? t.recording(seconds)
              : tooLong
                ? t.tooLong(bytes, max)
                : bytes === 0
                  ? TOUCH
                    ? ""
                    : t.composerHint
                  : packets === 1
                    ? t.onePacket(bytes)
                    : t.packets(bytes, packets))}
        </span>
        {onSendFile && (
          <>
            <input ref={photoInput} type="file" accept="image/*" hidden onChange={pickPhoto} />
            <button
              className="attach"
              onClick={() => photoInput.current?.click()}
              disabled={sending || recording !== null}
              title={t.attachPhoto}
              aria-label={t.attachPhoto}
            >
              <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden>
                <path
                  fill="currentColor"
                  d="M9 4 7.2 6H4a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-3.2L15 4H9zm3 4.5a4.5 4.5 0 1 1 0 9 4.5 4.5 0 0 1 0-9zm0 2a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z"
                />
              </svg>
            </button>
            {canRecord() && (
              <button
                className={recording ? "attach recording" : "attach"}
                onClick={recording ? stopRecording : record}
                disabled={sending}
                title={recording ? t.stopRecording : t.recordVoice}
                aria-label={recording ? t.stopRecording : t.recordVoice}
              >
                <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden>
                  {recording ? (
                    <rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor" />
                  ) : (
                    <path
                      fill="currentColor"
                      d="M12 14a3 3 0 0 0 3-3V5a3 3 0 0 0-6 0v6a3 3 0 0 0 3 3zm5-3a5 5 0 0 1-10 0H5a7 7 0 0 0 6 6.9V21h2v-3.1A7 7 0 0 0 19 11h-2z"
                    />
                  )}
                </svg>
              </button>
            )}
          </>
        )}
        <button className="primary" onClick={send} disabled={bytes === 0 || tooLong || sending || recording !== null}>
          {t.send}
        </button>
      </div>
    </div>
  );
}

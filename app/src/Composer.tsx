import { ImageIcon, MicrophoneIcon, PaperPlaneRightIcon, SpeakerHighIcon, StopIcon } from "@phosphor-icons/react";
import { useEffect, useLayoutEffect, useRef, useState, type ChangeEvent, type KeyboardEvent } from "react";
import { maxMessageBytes, onePacketBytes, radioPackets, utf8Length } from "./format";
import { useI18n } from "./i18n";
import { MAX_RECORDING_SECS, canRecord, shrinkPhoto, startRecording, type Recorder } from "./media";
import { playMessage } from "./sound";

type Props = {
  placeholder: string;
  /** Bytes a bundle adds around the text, which sets the radio packet count. */
  overhead: number;
  /** Clears the draft when it changes: the conversation it belongs to. */
  conversation: string;
  /** Sends a message; gives its id where it can also go as sound. */
  onSend: (text: string) => Promise<number | void>;
  /** Sends a photo or voice message. Without it there are no buttons for them. */
  onSendFile?: (mime: string, bytes: Uint8Array) => Promise<void>;
  /** Offers to send messages as sound too, to a device right next to this one. */
  sound?: boolean;
};

type Recording = Recorder & { started: number };

/** A touch keyboard's Enter starts a new line; the Send button sends. */
const TOUCH = window.matchMedia("(pointer: coarse)").matches;

export function Composer({ placeholder, overhead, conversation, onSend, onSendFile, sound = false }: Props) {
  const { t } = useI18n();
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [recording, setRecording] = useState<Recording | null>(null);
  const [seconds, setSeconds] = useState(0);
  // Sending by sound too, for this conversation, and how many messages are still being played.
  const [bySound, setBySound] = useState(false);
  const [playing, setPlaying] = useState(0);
  const photoInput = useRef<HTMLInputElement>(null);
  const field = useRef<HTMLTextAreaElement>(null);
  const recordingRef = useRef<Recording | null>(null);
  recordingRef.current = recording;
  // Counts the conversations this composer has left, so a microphone that
  // finishes opening after the move knows it is too late.
  const left = useRef(0);
  // The microphone is opening for a recording.
  const opening = useRef(false);

  // A recording belongs to the conversation it started in: moving to another
  // one, or leaving, throws it away and turns the microphone off, so it can
  // never go to someone else.
  useEffect(() => {
    setDraft("");
    setError(null);
    setBySound(false);
    return () => {
      left.current += 1;
      recordingRef.current?.cancel();
      recordingRef.current = null;
      setRecording(null);
    };
  }, [conversation]);

  // The field grows with the text, up to about five lines.
  useLayoutEffect(() => {
    const element = field.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, 132)}px`;
  }, [draft]);

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
    // A second press while the microphone opens would start a recording that nothing shows or stops.
    if (opening.current || recordingRef.current) return;
    opening.current = true;
    setError(null);
    const here = left.current;
    try {
      const started = await startRecording();
      // The conversation changed, or closed, while the microphone opened.
      if (left.current !== here) {
        started.cancel();
        return;
      }
      setSeconds(0);
      setRecording({ ...started, started: Date.now() });
    } catch {
      setError(t.micDenied);
    } finally {
      opening.current = false;
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
      const id = await onSend(draft);
      setDraft("");
      if (bySound && typeof id === "number") {
        setPlaying((count) => count + 1);
        playMessage(id)
          .catch((failure) => setError(`${t.soundMessageFailed} ${String(failure).replace(/^Error: /, "")}`))
          .finally(() => setPlaying((count) => count - 1));
      }
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

  const note =
    error ??
    (recording
      ? t.recording(seconds)
      : playing > 0
        ? t.soundSending
        : tooLong
          ? t.tooLong(bytes, max)
          : bytes > onePacketBytes(overhead)
            ? packets === 1
              ? t.onePacket(bytes)
              : t.packets(bytes, packets)
            : bySound
              ? t.soundModeNote
              : null);
  const empty = bytes === 0;

  return (
    <div className="composer">
      {note && <p className={error || tooLong ? "meter error" : recording ? "meter recording" : "meter"}>{note}</p>}
      <div className="composer-row">
        {onSendFile && (
          <>
            <input ref={photoInput} type="file" accept="image/*" hidden onChange={pickPhoto} />
            <button
              className="icon-button attach"
              onClick={() => photoInput.current?.click()}
              disabled={sending || recording !== null}
              title={t.attachPhoto}
              aria-label={t.attachPhoto}
            >
              <ImageIcon size={24} />
            </button>
          </>
        )}
        {sound && (
          <button
            className={bySound ? "icon-button attach on" : "icon-button attach"}
            onClick={() => setBySound((on) => !on)}
            aria-pressed={bySound}
            title={t.soundBySound}
            aria-label={t.soundBySound}
          >
            <SpeakerHighIcon size={24} weight={bySound ? "fill" : "regular"} />
          </button>
        )}
        <textarea
          ref={field}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={onKeyDown}
          placeholder={placeholder}
          title={TOUCH ? undefined : t.composerHint}
          rows={1}
          aria-label={placeholder}
        />
        {(empty || recording !== null) && onSendFile && canRecord() ? (
          <button
            className={recording ? "send recording" : "send"}
            onClick={recording ? stopRecording : record}
            disabled={sending}
            title={recording ? t.stopRecording : t.recordVoice}
            aria-label={recording ? t.stopRecording : t.recordVoice}
          >
            {recording ? <StopIcon size={20} weight="fill" /> : <MicrophoneIcon size={21} weight="fill" />}
          </button>
        ) : (
          <button
            className="send"
            onClick={send}
            disabled={empty || tooLong || sending || recording !== null}
            title={t.send}
            aria-label={t.send}
          >
            <PaperPlaneRightIcon size={20} weight="fill" />
          </button>
        )}
      </div>
    </div>
  );
}

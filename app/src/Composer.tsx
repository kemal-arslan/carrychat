import { useEffect, useState, type KeyboardEvent } from "react";
import { maxMessageBytes, onePacketBytes, radioPackets, utf8Length } from "./format";
import { useI18n } from "./i18n";

type Props = {
  placeholder: string;
  /** Bytes a bundle adds around the text, which sets the radio packet count. */
  overhead: number;
  /** Clears the draft when it changes: the conversation it belongs to. */
  conversation: string;
  onSend: (text: string) => Promise<void>;
};

export function Composer({ placeholder, overhead, conversation, onSend }: Props) {
  const { t } = useI18n();
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);

  useEffect(() => {
    setDraft("");
    setError(null);
  }, [conversation]);

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
            (tooLong
              ? t.tooLong(bytes, max)
              : bytes === 0
                ? t.composerHint
                : packets === 1
                  ? t.onePacket(bytes)
                  : t.packets(bytes, packets))}
        </span>
        <button className="primary" onClick={send} disabled={bytes === 0 || tooLong || sending}>
          {t.send}
        </button>
      </div>
    </div>
  );
}

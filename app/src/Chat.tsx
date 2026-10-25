import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { Contact, Message } from "./api";
import { MAX_MESSAGE_BYTES, ONE_PACKET_BYTES, fullTime, radioPackets, shortTime, utf8Length } from "./format";
import { useI18n } from "./i18n";

type Props = {
  contact: Contact;
  messages: Message[];
  onSend: (text: string) => Promise<void>;
  onAccept: () => void;
  onDecline: () => void;
};

export function Chat({ contact, messages, onSend, onAccept, onDecline }: Props) {
  const { t, language } = useI18n();
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const list = useRef<HTMLOListElement>(null);

  useEffect(() => {
    list.current?.scrollTo({ top: list.current.scrollHeight });
  }, [messages, contact.id]);

  useEffect(() => {
    setDraft("");
    setError(null);
  }, [contact.id]);

  const bytes = utf8Length(draft.trim());
  const tooLong = bytes > MAX_MESSAGE_BYTES;
  const packets = radioPackets(bytes);

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

  function state(message: Message) {
    if (message.read) return ["read", t.read];
    if (message.delivered) return ["delivered", t.delivered];
    return ["waiting", t.waitingForPath];
  }

  return (
    <section className="chat" aria-label={contact.name}>
      <header className="chat-header">
        <h2>{contact.name}</h2>
        <span className="fingerprint" title={t.compareFingerprint}>
          {contact.fingerprint}
        </span>
        {!contact.request && (
          <span
            className={contact.secure ? "secrecy on" : "secrecy"}
            title={contact.secure ? t.secureHint : t.securePendingHint}
          >
            {contact.secure ? t.secure : t.securePending}
          </span>
        )}
      </header>

      <ol className="messages" ref={list}>
        {messages.length === 0 && <li className="messages-empty">{t.chatEmpty(contact.name)}</li>}
        {messages.map((message) => {
          const [stateClass, stateLabel] = state(message);
          return (
            <li key={message.id} className={message.outgoing ? "bubble outgoing" : "bubble incoming"}>
              <p>{message.body}</p>
              <footer>
                <time title={fullTime(message.createdAt, language)}>{shortTime(message.createdAt, language)}</time>
                {message.outgoing && <span className={`state ${stateClass}`}>{stateLabel}</span>}
              </footer>
            </li>
          );
        })}
      </ol>

      {contact.request ? (
        <div className="request-bar">
          <p>{t.requestBar(contact.name)}</p>
          <p className="fingerprint large">{contact.fingerprint}</p>
          <div className="dialog-actions">
            <button onClick={onDecline}>{t.decline}</button>
            <button className="primary" onClick={onAccept}>
              {t.acceptAndReply}
            </button>
          </div>
        </div>
      ) : (
        <div className="composer">
          <textarea
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={onKeyDown}
            placeholder={t.messageTo(contact.name)}
            rows={2}
            aria-label={t.messageTo(contact.name)}
          />
          <div className="composer-row">
            <span className={tooLong ? "meter error" : bytes > ONE_PACKET_BYTES ? "meter warn" : "meter"}>
              {error ??
                (tooLong
                  ? t.tooLong(bytes, MAX_MESSAGE_BYTES)
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
      )}
    </section>
  );
}

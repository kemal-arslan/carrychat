import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { Contact, Message } from "./api";
import { MAX_MESSAGE_BYTES, ONE_PACKET_BYTES, fullTime, radioPackets, shortTime, utf8Length } from "./format";

type Props = {
  contact: Contact;
  messages: Message[];
  onSend: (text: string) => Promise<void>;
  onAccept: () => void;
  onDecline: () => void;
};

export function Chat({ contact, messages, onSend, onAccept, onDecline }: Props) {
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

  async function send() {
    if (bytes === 0 || tooLong || sending) return;
    setSending(true);
    setError(null);
    try {
      await onSend(draft);
      setDraft("");
    } catch (failure) {
      setError(String(failure));
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
    <section className="chat" aria-label={`Conversation with ${contact.name}`}>
      <header className="chat-header">
        <h2>{contact.name}</h2>
        <span className="fingerprint" title="Compare this with what they see under My card">
          {contact.fingerprint}
        </span>
      </header>

      <ol className="messages" ref={list}>
        {messages.length === 0 && (
          <li className="messages-empty">
            Nothing here yet. What you write waits on this computer until a path to {contact.name} appears: another
            Kervan computer on the network, or a file someone carries.
          </li>
        )}
        {messages.map((message) => (
          <li key={message.id} className={message.outgoing ? "bubble outgoing" : "bubble incoming"}>
            <p>{message.body}</p>
            <footer>
              <time title={fullTime(message.createdAt)}>{shortTime(message.createdAt)}</time>
              {message.outgoing && (
                <span className={message.delivered ? "state delivered" : "state waiting"}>
                  {message.delivered ? "Delivered" : "Waiting for a path"}
                </span>
              )}
            </footer>
          </li>
        ))}
      </ol>

      {contact.request ? (
        <div className="request-bar">
          <p>
            <strong>{contact.name}</strong> has your card and wrote to you first. Check this fingerprint with them
            before you accept:
          </p>
          <p className="fingerprint large">{contact.fingerprint}</p>
          <div className="dialog-actions">
            <button onClick={onDecline}>Decline</button>
            <button className="primary" onClick={onAccept}>
              Accept and reply
            </button>
          </div>
        </div>
      ) : (
        <div className="composer">
          <textarea
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={onKeyDown}
            placeholder={`Message ${contact.name}`}
            rows={2}
            aria-label="Message"
          />
          <div className="composer-row">
            <span className={tooLong ? "meter error" : bytes > ONE_PACKET_BYTES ? "meter warn" : "meter"}>
              {error ??
                (tooLong
                  ? `Too long: ${bytes} of ${MAX_MESSAGE_BYTES} bytes`
                  : bytes === 0
                    ? "Enter sends, Shift+Enter starts a new line"
                    : `${bytes} bytes · ${radioPackets(bytes) === 1 ? "fits one radio packet" : `${radioPackets(bytes)} radio packets`}`)}
            </span>
            <button className="primary" onClick={send} disabled={bytes === 0 || tooLong || sending}>
              Send
            </button>
          </div>
        </div>
      )}
    </section>
  );
}

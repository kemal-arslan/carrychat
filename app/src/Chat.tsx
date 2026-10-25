import { useEffect, useRef } from "react";
import type { Contact, Message } from "./api";
import { Composer } from "./Composer";
import { PAIR_OVERHEAD, fullTime, shortTime } from "./format";
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
  const list = useRef<HTMLOListElement>(null);

  useEffect(() => {
    list.current?.scrollTo({ top: list.current.scrollHeight });
  }, [messages, contact.id]);

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
        <Composer
          placeholder={t.messageTo(contact.name)}
          overhead={PAIR_OVERHEAD}
          conversation={`contact-${contact.id}`}
          onSend={onSend}
        />
      )}
    </section>
  );
}

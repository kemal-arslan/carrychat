import { CheckIcon, ChecksIcon, HourglassMediumIcon, ShieldCheckIcon, ShieldIcon } from "@phosphor-icons/react";
import { Fragment, useRef } from "react";
import type { Contact, Message } from "./api";
import { Composer } from "./Composer";
import { FileBubble } from "./FileBubble";
import { PAIR_OVERHEAD, clockTime, fullTime } from "./format";
import { useI18n } from "./i18n";
import { ChatHeader, bubbleClass, useStickToBottom, useThreadLayout } from "./Thread";

type Props = {
  contact: Contact;
  messages: Message[];
  onSend: (text: string) => Promise<void>;
  onSendFile: (mime: string, bytes: Uint8Array) => Promise<void>;
  onAccept: () => void;
  onDecline: () => void;
  /** Shows the contact's fingerprint and how private the conversation is. */
  onInfo: () => void;
  /** Set on a phone, where the conversation covers the list. */
  onBack?: () => void;
};

export function Chat({ contact, messages, onSend, onSendFile, onAccept, onDecline, onInfo, onBack }: Props) {
  const { t, language } = useI18n();
  const list = useRef<HTMLOListElement>(null);
  const layout = useThreadLayout(messages);

  useStickToBottom(list, contact.id, messages[messages.length - 1]?.id);

  function state(message: Message) {
    if (message.read)
      return (
        <span className="state" title={t.read} aria-label={t.read}>
          <ChecksIcon size={15} weight="bold" />
        </span>
      );
    if (message.delivered)
      return (
        <span className="state" title={t.delivered} aria-label={t.delivered}>
          <CheckIcon size={15} weight="bold" />
        </span>
      );
    return (
      <span className="state waiting">
        <HourglassMediumIcon size={13} weight="bold" />
        {t.waitingForPath}
      </span>
    );
  }

  return (
    <section className="chat" aria-label={contact.name}>
      <ChatHeader
        name={contact.name}
        kind="contact"
        onBack={onBack}
        onTitle={onInfo}
        sub={
          contact.request ? (
            t.requestTag
          ) : (
            <>
              {contact.secure ? <ShieldCheckIcon size={13} weight="fill" className="secure" /> : <ShieldIcon size={13} />}
              {contact.secure ? t.secure : t.securePending}
            </>
          )
        }
      />

      <ol className="messages" ref={list}>
        {messages.length === 0 && <li className="messages-empty">{t.chatEmpty(contact.name)}</li>}
        {layout.map(({ item: message, day, first, tail }) => (
          <Fragment key={message.id}>
            {day && <li className="day">{day}</li>}
            <li
              className={bubbleClass(
                message.outgoing,
                first,
                tail,
                message.outgoing && !message.delivered && !message.read ? "waiting" : undefined,
              )}
            >
              {message.file && <FileBubble file={message.file} />}
              {message.body && <p>{message.body}</p>}
              <footer>
                <time title={fullTime(message.createdAt, language)}>{clockTime(message.createdAt, language)}</time>
                {message.outgoing && state(message)}
              </footer>
            </li>
          </Fragment>
        ))}
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
          onSendFile={onSendFile}
        />
      )}
    </section>
  );
}

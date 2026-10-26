import { useEffect, useRef } from "react";
import type { Group, GroupMessage } from "./api";
import { BackButton } from "./BackButton";
import { Composer } from "./Composer";
import { FileBubble } from "./FileBubble";
import { GROUP_OVERHEAD, fullTime, shortTime } from "./format";
import { useI18n } from "./i18n";

type Props = {
  group: Group;
  messages: GroupMessage[];
  onSend: (text: string) => Promise<void>;
  onSendFile: (mime: string, bytes: Uint8Array) => Promise<void>;
  onMembers: () => void;
  onBack?: () => void;
};

export function GroupChat({ group, messages, onSend, onSendFile, onMembers, onBack }: Props) {
  const { t, language } = useI18n();
  const list = useRef<HTMLOListElement>(null);

  useEffect(() => {
    list.current?.scrollTo({ top: list.current.scrollHeight });
  }, [messages, group.id]);

  return (
    <section className="chat" aria-label={group.name}>
      <header className="chat-header">
        {onBack && <BackButton onBack={onBack} />}
        <h2>{group.name}</h2>
        <button className="members-link" onClick={onMembers}>
          {t.membersCount(group.members.length)}
        </button>
        <span className="secrecy on" title={t.groupSecureHint}>
          {t.groupSecure}
        </span>
      </header>

      <ol className="messages" ref={list}>
        {messages.length === 0 && <li className="messages-empty">{t.groupChatEmpty(group.name)}</li>}
        {messages.map((message) => (
          <li key={message.id} className={message.outgoing ? "bubble outgoing" : "bubble incoming"}>
            {message.sender && <span className="sender">{message.sender}</span>}
            {message.file && <FileBubble file={message.file} />}
            {message.body && <p>{message.body}</p>}
            <footer>
              <time title={fullTime(message.createdAt, language)}>{shortTime(message.createdAt, language)}</time>
            </footer>
          </li>
        ))}
      </ol>

      {group.member ? (
        <Composer
          placeholder={t.messageGroup(group.name)}
          overhead={GROUP_OVERHEAD}
          conversation={`group-${group.id}`}
          onSend={onSend}
          onSendFile={onSendFile}
        />
      ) : (
        <div className="request-bar">
          <p>{t.leftGroup}</p>
        </div>
      )}
    </section>
  );
}

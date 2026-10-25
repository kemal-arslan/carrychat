import { useEffect, useRef } from "react";
import type { Group, GroupMessage } from "./api";
import { Composer } from "./Composer";
import { GROUP_OVERHEAD, fullTime, shortTime } from "./format";
import { useI18n } from "./i18n";

type Props = {
  group: Group;
  messages: GroupMessage[];
  onSend: (text: string) => Promise<void>;
  onMembers: () => void;
};

export function GroupChat({ group, messages, onSend, onMembers }: Props) {
  const { t, language } = useI18n();
  const list = useRef<HTMLOListElement>(null);

  useEffect(() => {
    list.current?.scrollTo({ top: list.current.scrollHeight });
  }, [messages, group.id]);

  return (
    <section className="chat" aria-label={group.name}>
      <header className="chat-header">
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
            <p>{message.body}</p>
            <footer>
              <time title={fullTime(message.createdAt, language)}>{shortTime(message.createdAt, language)}</time>
            </footer>
          </li>
        ))}
      </ol>

      {group.member ? (
        <Composer
          placeholder={t.messageTo(group.name)}
          overhead={GROUP_OVERHEAD}
          conversation={`group-${group.id}`}
          onSend={onSend}
        />
      ) : (
        <div className="request-bar">
          <p>{t.leftGroup}</p>
        </div>
      )}
    </section>
  );
}

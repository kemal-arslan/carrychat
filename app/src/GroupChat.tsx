import { DotsThreeVerticalIcon } from "@phosphor-icons/react";
import { Fragment, useEffect, useRef } from "react";
import type { Group, GroupMessage } from "./api";
import { Composer } from "./Composer";
import { FileBubble } from "./FileBubble";
import { GROUP_OVERHEAD, fullTime, shortTime } from "./format";
import { useI18n } from "./i18n";
import { ChatHeader, bubbleClass, useThreadLayout } from "./Thread";

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
  const layout = useThreadLayout(messages);

  useEffect(() => {
    list.current?.scrollTo({ top: list.current.scrollHeight });
  }, [messages, group.id]);

  return (
    <section className="chat" aria-label={group.name}>
      <ChatHeader
        name={group.name}
        kind="group"
        onBack={onBack}
        onTitle={onMembers}
        sub={`${t.membersCount(group.members.length)} · ${t.groupSecure}`}
      >
        <button className="icon-button" onClick={onMembers} aria-label={t.membersTitle(group.name)} title={t.membersTitle(group.name)}>
          <DotsThreeVerticalIcon size={22} weight="bold" />
        </button>
      </ChatHeader>

      <ol className="messages" ref={list}>
        {messages.length === 0 && <li className="messages-empty">{t.groupChatEmpty(group.name)}</li>}
        {layout.map(({ item: message, day, first, tail }) => (
          <Fragment key={message.id}>
            {day && <li className="day">{day}</li>}
            <li className={bubbleClass(message.outgoing, first, tail)}>
              {message.sender && first && <span className="sender">{message.sender}</span>}
              {message.file && <FileBubble file={message.file} />}
              {message.body && <p>{message.body}</p>}
              <footer>
                <time title={fullTime(message.createdAt, language)}>{shortTime(message.createdAt, language)}</time>
              </footer>
            </li>
          </Fragment>
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

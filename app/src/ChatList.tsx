import { CaretRightIcon, HourglassMediumIcon, MagnifyingGlassIcon, QrCodeIcon, XIcon } from "@phosphor-icons/react";
import { useState } from "react";
import type { Channel, Contact, Group, Status } from "./api";
import { Avatar } from "./Avatar";
import { shortTime } from "./format";
import { useI18n } from "./i18n";
import { summary } from "./links";

type Kind = "contact" | "group" | "channel";

type Item = {
  kind: Kind;
  id: number;
  name: string;
  preview: string;
  lastAt: number | null;
  unread: number;
  request: boolean;
  /** Our messages here that have not arrived yet. */
  waiting: number;
};

type Filter = "all" | "unread" | "waiting" | "group" | "channel";

type Props = {
  contacts: Contact[];
  groups: Group[];
  channels: Channel[];
  status: Status | null;
  /** The open conversation, highlighted where the list stays in view. */
  active: { kind: Kind; id: number } | null;
  onOpen: (kind: Kind, id: number) => void;
  onNetwork: () => void;
  onCard: () => void;
};

/** Every conversation in one list: people, groups and channels, newest first. */
export function ChatList({ contacts, groups, channels, status, active, onOpen, onNetwork, onCard }: Props) {
  const { t, language } = useI18n();
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState<string | null>(null);

  const items: Item[] = [
    // A contact shows here once there is something to read; all of them are under People.
    ...contacts
      .filter((contact) => contact.request || contact.lastAt !== null)
      .map((contact) => ({
        kind: "contact" as const,
        id: contact.id,
        name: contact.name,
        preview: contact.lastMessage === null ? t.noMessagesYet : contact.lastMessage || t.file,
        lastAt: contact.lastAt,
        unread: contact.unread,
        request: contact.request,
        waiting: contact.waiting,
      })),
    ...groups.map((group) => ({
      kind: "group" as const,
      id: group.id,
      name: group.name,
      preview: group.lastMessage === null ? t.membersCount(group.members.length) : group.lastMessage || t.file,
      lastAt: group.lastAt,
      unread: group.unread,
      request: false,
      waiting: 0,
    })),
    ...channels.map((channel) => ({
      kind: "channel" as const,
      id: channel.id,
      name: channel.name,
      preview: channel.lastMessage === null ? t.postingAs(channel.nick) : channel.lastMessage || t.file,
      lastAt: channel.lastAt,
      unread: channel.unread,
      request: false,
      waiting: 0,
    })),
  ].sort(
    (a, b) =>
      Number(b.request) - Number(a.request) ||
      (b.lastAt ?? -1) - (a.lastAt ?? -1) ||
      a.name.localeCompare(b.name, language),
  );

  const needle = query?.trim().toLocaleLowerCase(language) ?? "";
  const shown = items.filter(
    (item) =>
      (filter === "all" ||
        (filter === "unread"
          ? item.unread > 0 || item.request
          : filter === "waiting"
            ? item.waiting > 0
            : item.kind === filter)) &&
      (needle === "" || item.name.toLocaleLowerCase(language).includes(needle)),
  );
  const unreadCount = items.filter((item) => item.unread > 0 || item.request).length;
  const waitingCount = items.filter((item) => item.waiting > 0).length;
  const line = summary(status, t);

  const chips: { filter: Filter; label: string; count?: number }[] = [
    { filter: "all", label: t.filterAll },
    { filter: "unread", label: t.filterUnread, count: unreadCount },
    { filter: "waiting", label: t.filterWaiting, count: waitingCount },
    { filter: "group", label: t.groups },
    { filter: "channel", label: t.channels },
  ];

  return (
    <div className="tab-page">
      <header className="page-head">
        <h1 className="page-title">{t.tabChats}</h1>
        <div className="page-actions">
          <button
            className="icon-button"
            onClick={() => setQuery(query === null ? "" : null)}
            aria-label={t.search}
            title={t.search}
          >
            {query === null ? <MagnifyingGlassIcon size={22} /> : <XIcon size={22} />}
          </button>
          <button className="icon-button" onClick={onCard} aria-label={t.myCard} title={t.myCard}>
            <QrCodeIcon size={22} />
          </button>
        </div>
      </header>

      {query !== null && (
        <input
          className="search"
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t.search}
          aria-label={t.search}
          autoFocus
        />
      )}

      <button className={line.problem ? "summary trouble" : "summary"} onClick={onNetwork}>
        <span className={line.live ? "dot live" : "dot"} aria-hidden />
        <span>{line.text}</span>
        <CaretRightIcon size={13} weight="bold" />
      </button>

      <div className="chips" role="tablist">
        {chips.map((chip) => (
          <button
            key={chip.filter}
            role="tab"
            aria-selected={filter === chip.filter}
            className={filter === chip.filter ? "chip on" : "chip"}
            onClick={() => setFilter(chip.filter)}
          >
            {chip.label}
            {chip.count !== undefined && chip.count > 0 && <span className="count">{chip.count}</span>}
          </button>
        ))}
      </div>

      <div className="rows">
        {shown.map((item) => {
          const isActive = active?.kind === item.kind && active.id === item.id;
          return (
            <button
              key={`${item.kind}-${item.id}`}
              className={isActive ? "chat-row active" : "chat-row"}
              onClick={() => onOpen(item.kind, item.id)}
            >
              <Avatar name={item.name} kind={item.kind} />
              <span className="row-main">
                <span className="row-top">
                  <span className="name">{item.name}</span>
                  {item.request ? (
                    <span className="tag accent">{t.requestTag}</span>
                  ) : (
                    item.lastAt !== null && (
                      <time className={item.unread > 0 && !isActive ? "new" : undefined}>
                        {shortTime(item.lastAt, language)}
                      </time>
                    )
                  )}
                </span>
                <span className="row-bottom">
                  {item.waiting > 0 && (
                    <HourglassMediumIcon className="waiting-mark" size={14} weight="bold" aria-label={t.onTheWay} />
                  )}
                  <span className="preview">{item.preview}</span>
                  {item.unread > 0 && !isActive && <span className="badge">{item.unread}</span>}
                </span>
              </span>
            </button>
          );
        })}
        {shown.length === 0 && <p className="empty">{items.length === 0 ? t.chatsEmpty : t.nothingFound}</p>}
      </div>
    </div>
  );
}

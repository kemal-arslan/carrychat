import type { ReactNode } from "react";
import { Avatar } from "./Avatar";
import { BackButton } from "./BackButton";
import { useI18n, type Language } from "./i18n";

type HeaderProps = {
  name: string;
  kind: "contact" | "group" | "channel";
  sub: ReactNode;
  onBack?: () => void;
  /** Opens details about the conversation when its name is tapped. */
  onTitle?: () => void;
  children?: ReactNode;
};

/** The top of a conversation: back, who it is with, one line about it. */
export function ChatHeader({ name, kind, sub, onBack, onTitle, children }: HeaderProps) {
  return (
    <header className="chat-header">
      {onBack && <BackButton onBack={onBack} />}
      <button className="head-main" onClick={onTitle} disabled={!onTitle}>
        <Avatar name={name} kind={kind} size={40} />
        <span className="head-text">
          <span className="head-name" role="heading" aria-level={2}>
            {kind === "channel" ? `#${name}` : name}
          </span>
          <span className="head-sub">{sub}</span>
        </span>
      </button>
      {children}
    </header>
  );
}

type Item = { outgoing: boolean; createdAt: number; sender?: string | null };

/** Bubbles from one side within this many seconds read as one run. */
const RUN_GAP = 5 * 60;

function sameDay(a: number, b: number): boolean {
  return new Date(a * 1000).toDateString() === new Date(b * 1000).toDateString();
}

function sameRun(a: Item, b: Item): boolean {
  return (
    a.outgoing === b.outgoing &&
    (a.sender ?? null) === (b.sender ?? null) &&
    sameDay(a.createdAt, b.createdAt) &&
    Math.abs(b.createdAt - a.createdAt) < RUN_GAP
  );
}

function dayLabel(unix: number, language: Language, today: string, yesterday: string): string {
  const date = new Date(unix * 1000);
  const now = new Date();
  if (date.toDateString() === now.toDateString()) return today;
  if (date.toDateString() === new Date(now.getTime() - 86_400_000).toDateString()) return yesterday;
  return new Intl.DateTimeFormat(language, {
    day: "numeric",
    month: "long",
    year: date.getFullYear() === now.getFullYear() ? undefined : "numeric",
  }).format(date);
}

/** Where each bubble sits: a day heading before it, and whether it starts or ends a run. */
export function useThreadLayout<T extends Item>(items: T[]) {
  const { t, language } = useI18n();
  return items.map((item, index) => {
    const previous = items[index - 1];
    const next = items[index + 1];
    return {
      item,
      day: !previous || !sameDay(previous.createdAt, item.createdAt) ? dayLabel(item.createdAt, language, t.today, t.yesterday) : null,
      first: !previous || !sameRun(previous, item),
      tail: !next || !sameRun(item, next),
    };
  });
}

export function bubbleClass(outgoing: boolean, first: boolean, tail: boolean, extra?: string): string {
  return ["bubble", outgoing ? "outgoing" : "incoming", first && "first", tail && "tail", extra].filter(Boolean).join(" ");
}

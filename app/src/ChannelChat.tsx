import { useEffect, useRef } from "react";
import type { Channel, Post } from "./api";
import { BackButton } from "./BackButton";
import { Composer } from "./Composer";
import { FileBubble } from "./FileBubble";
import { CHANNEL_OVERHEAD, fullTime, shortTime, utf8Length } from "./format";
import { useI18n } from "./i18n";

type Props = {
  channel: Channel;
  posts: Post[];
  onSend: (text: string) => Promise<void>;
  onSendFile: (mime: string, bytes: Uint8Array) => Promise<void>;
  onSettings: () => void;
  onMute: (post: Post) => void;
  onAddCard: (card: string) => void;
  onBack?: () => void;
};

export function ChannelChat({ channel, posts, onSend, onSendFile, onSettings, onMute, onAddCard, onBack }: Props) {
  const { t, language } = useI18n();
  const list = useRef<HTMLOListElement>(null);

  useEffect(() => {
    list.current?.scrollTo({ top: list.current.scrollHeight });
  }, [posts, channel.id]);

  return (
    <section className="chat" aria-label={`#${channel.name}`}>
      <header className="chat-header">
        {onBack && <BackButton onBack={onBack} />}
        <h2>#{channel.name}</h2>
        <button className="members-link" onClick={onSettings}>
          {t.postingAs(channel.nick)}
        </button>
        <span className="secrecy" title={t.channelPublicHint}>
          {channel.hasPassword ? t.channelProtected : t.channelPublic}
        </span>
      </header>

      <ol className="messages" ref={list}>
        {posts.length === 0 && <li className="messages-empty">{t.channelEmpty(channel.name)}</li>}
        {posts.map((post) => (
          <li key={post.id} className={post.outgoing ? "bubble outgoing" : "bubble incoming"}>
            {!post.outgoing && (
              <span className="sender">
                {post.nick} <span className="author-id">{post.author}</span>
              </span>
            )}
            {post.sharedCard ? (
              <div className="shared-card">
                <p>{post.outgoing ? t.youSharedCard : t.sharedCard(post.sharedCard.name)}</p>
                <p className="fingerprint">{post.sharedCard.fingerprint}</p>
                {!post.outgoing && (
                  <button onClick={() => onAddCard(post.sharedCard!.card)}>{t.addThisContact}</button>
                )}
              </div>
            ) : (
              <>
                {post.file && <FileBubble file={post.file} />}
                {post.body && <p>{post.body}</p>}
              </>
            )}
            <footer>
              <time title={fullTime(post.createdAt, language)}>{shortTime(post.createdAt, language)}</time>
              {!post.outgoing && (
                <button className="mute" onClick={() => onMute(post)}>
                  {t.mute}
                </button>
              )}
            </footer>
          </li>
        ))}
      </ol>

      <Composer
        placeholder={t.messageChannel(channel.name)}
        overhead={CHANNEL_OVERHEAD + utf8Length(channel.nick)}
        conversation={`channel-${channel.id}`}
        onSend={onSend}
        onSendFile={onSendFile}
      />
    </section>
  );
}

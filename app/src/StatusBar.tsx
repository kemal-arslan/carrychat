import type { Status } from "./api";
import { plural } from "./format";

type Props = {
  status: Status | null;
  preview: boolean;
  onConnect: () => void;
  onImport: () => void;
  onExport: () => void;
};

export function StatusBar({ status, preview, onConnect, onImport, onExport }: Props) {
  const link = !status || status.lanPort === null ? "off" : status.lanPeers > 0 ? "up" : "waiting";
  const label = {
    off: "Local network: off",
    waiting: "Local network: looking for other computers",
    up: `Local network: ${plural(status?.lanPeers ?? 0, "computer", "computers")} nearby`,
  }[link];

  return (
    <footer className="status">
      <span className={`link ${link}`}>
        <span className="dot" aria-hidden />
        {label}
      </span>
      {status && (
        <>
          <span>{plural(status.outbox, "message", "messages")} waiting to send</span>
          <span>carrying {status.carried} for others</span>
        </>
      )}
      {preview && <span className="preview-tag">Browser preview with sample data</span>}
      <span className="status-actions">
        <button onClick={onConnect}>Connect…</button>
        <button onClick={onImport}>Import file…</button>
        <button onClick={onExport}>Export file…</button>
      </span>
    </footer>
  );
}

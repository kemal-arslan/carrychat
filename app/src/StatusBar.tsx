import type { Status } from "./api";
import { useI18n } from "./i18n";

type Props = {
  status: Status | null;
  preview: boolean;
  onNetworkHelp: () => void;
  onConnect: () => void;
  onImport: () => void;
  onExport: () => void;
};

export function StatusBar({ status, preview, onNetworkHelp, onConnect, onImport, onExport }: Props) {
  const { t } = useI18n();
  const link =
    !status || status.lanPort === null
      ? "off"
      : status.lanProblem
        ? status.lanProblem.kind
        : status.lanPeers > 0
          ? "up"
          : "waiting";
  const label = {
    off: t.lanOff,
    waiting: t.lanWaiting,
    up: t.lanUp(status?.lanPeers ?? 0),
    blocked: t.lanBlocked,
    failed: t.lanFailed,
  }[link];

  const tor = status?.internetTor ?? null;
  const internet = !status?.internetOn
    ? "off"
    : tor?.state === "failed"
      ? "failed"
      : tor?.state !== "starting" && status.internetRelays > 0
        ? "up"
        : "waiting";
  const internetLabel = tor
    ? {
        off: t.internetOff,
        failed: t.torFailed,
        waiting: tor.state === "starting" ? t.torStarting : t.torConnecting,
        up: t.torUp(status?.internetRelays ?? 0),
      }[internet]
    : {
        off: t.internetOff,
        failed: t.internetOff,
        waiting: t.internetConnecting,
        up: t.internetUp(status?.internetRelays ?? 0),
      }[internet];
  const internetTitle = tor?.state === "failed" ? tor.detail : (status?.internetRefusal ?? undefined);

  const bluetooth = !status?.bluetoothOn ? "off" : status.bluetoothPeers > 0 ? "up" : "waiting";
  const bluetoothLabel = {
    off: t.bluetoothOff,
    waiting: t.bluetoothWaiting,
    up: t.bluetoothUp(status?.bluetoothPeers ?? 0),
  }[bluetooth];

  return (
    <footer className="status">
      <span className={`link ${link}`} title={status?.lanProblem?.detail}>
        <span className="dot" aria-hidden />
        {label}
        {(link === "blocked" || link === "failed") && (
          <button className="link-action" onClick={onNetworkHelp}>
            {link === "blocked" ? t.fix : t.details}
          </button>
        )}
      </span>
      <span className={`link ${internet}`} title={internetTitle}>
        <span className="dot" aria-hidden />
        {internetLabel}
      </span>
      <span className={`link ${bluetooth}`}>
        <span className="dot" aria-hidden />
        {bluetoothLabel}
      </span>
      {status && (
        <>
          <span>{t.waitingToSend(status.outbox)}</span>
          <span>{t.carrying(status.carried)}</span>
        </>
      )}
      {preview && <span className="preview-tag">{t.preview}</span>}
      <span className="status-actions">
        <button onClick={onConnect}>{t.connect}</button>
        <button onClick={onImport}>{t.importFile}</button>
        <button onClick={onExport}>{t.exportFile}</button>
      </span>
    </footer>
  );
}

import {
  BluetoothIcon,
  CaretRightIcon,
  DownloadSimpleIcon,
  ExportIcon,
  GlobeSimpleIcon,
  LinkSimpleIcon,
  ShieldCheckIcon,
  WifiHighIcon,
} from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { api, type Status } from "./api";
import { useI18n } from "./i18n";
import { bluetoothLabel, internetLabel, lanLabel, linkStates } from "./links";

type Props = {
  status: Status | null;
  preview: boolean;
  onNetworkHelp: () => void;
  onConnect: () => void;
  onImport: () => void;
  onExport: () => void;
};

/** The links a message can leave by, in place of a status bar. */
export function Network({ status, preview, onNetworkHelp, onConnect, onImport, onExport }: Props) {
  const { t } = useI18n();
  const { lan, bluetooth, internet, nearby } = linkStates(status);
  // Tor and Bluetooth as the node started with them, and as now chosen.
  const [atStart, setAtStart] = useState<{ tor: boolean; bluetooth: boolean } | null>(null);
  const [tor, setTor] = useState(false);
  const [bluetoothOn, setBluetoothOn] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.settings().then((settings) => {
      setAtStart({ tor: settings.tor, bluetooth: settings.bluetooth });
      setTor(settings.tor);
      setBluetoothOn(settings.bluetooth);
    });
  }, []);

  async function toggleTor(on: boolean) {
    setTor(on);
    await api.setTor(on).catch((failure) => setError(String(failure).replace(/^Error: /, "")));
  }

  async function toggleBluetooth(on: boolean) {
    setBluetoothOn(on);
    await api.setBluetooth(on).catch((failure) => setError(String(failure).replace(/^Error: /, "")));
  }

  const needsRestart = atStart !== null && (tor !== atStart.tor || bluetoothOn !== atStart.bluetooth);
  const lanTrouble = lan === "blocked" || lan === "failed";
  const hint =
    nearby > 0 ? (internet === "up" ? t.nearbyOnline : t.nearbyOffline) : internet === "up" ? t.internetOnly : t.waitingHere;

  return (
    <div className="tab-page">
      <header className="page-head">
        <h1 className="page-title">{t.tabNetwork}</h1>
      </header>
      <p className="page-sub">{t.networkSubtitle}</p>

      <div className="nearby-card">
        <div className={nearby > 0 ? "radar live" : "radar"} aria-hidden>
          <span className="ring outer" />
          <span className="ring inner" />
          <span className="ping" />
          <span className="me" />
          {Array.from({ length: Math.min(nearby, 5) }, (_, index) => (
            <span key={index} className={`peer p${index}`} />
          ))}
        </div>
        <div>
          <strong>{nearby > 0 ? t.nearbyCount(nearby) : t.nobodyNearby}</strong>
          <span>{hint}</span>
        </div>
      </div>

      <h2 className="group-label">{t.linksTitle}</h2>
      <div className="group">
        <div className="row-item" title={status?.lanProblem?.detail}>
          <span className="tile">
            <WifiHighIcon size={20} />
          </span>
          <span className="row-text">
            <b>{t.lanName}</b>
            <span className={lan === "up" ? "live" : lanTrouble ? "trouble" : undefined}>{lanLabel(lan, status, t)}</span>
          </span>
          {lanTrouble && (
            <button className="small" onClick={onNetworkHelp}>
              {lan === "blocked" ? t.fix : t.details}
            </button>
          )}
        </div>
        <label className="row-item toggle" title={t.bluetoothHint}>
          <span className="tile">
            <BluetoothIcon size={20} />
          </span>
          <span className="row-text">
            <b>{t.bluetoothName}</b>
            <span className={bluetooth === "up" ? "live" : undefined}>{bluetoothLabel(bluetooth, status, t)}</span>
          </span>
          <input
            type="checkbox"
            checked={bluetoothOn}
            disabled={atStart === null}
            onChange={(event) => toggleBluetooth(event.target.checked)}
            aria-label={t.bluetoothSetting}
          />
        </label>
        <div
          className="row-item"
          title={status?.internetTor?.state === "failed" ? status.internetTor.detail : (status?.internetRefusal ?? undefined)}
        >
          <span className="tile">
            <GlobeSimpleIcon size={20} />
          </span>
          <span className="row-text">
            <b>{t.internetName}</b>
            <span className={internet === "up" ? "live" : internet === "failed" ? "trouble" : undefined}>
              {internetLabel(internet, status, t)}
            </span>
          </span>
        </div>
        <label className="row-item toggle" title={t.torHint}>
          <span className="tile">
            <ShieldCheckIcon size={20} />
          </span>
          <span className="row-text">
            <b>{t.torName}</b>
            <span>{t.torShort}</span>
          </span>
          <input
            type="checkbox"
            checked={tor}
            disabled={atStart === null}
            onChange={(event) => toggleTor(event.target.checked)}
            aria-label={t.torSetting}
          />
        </label>
        {needsRestart && (
          <div className="row-item">
            <span className="row-text">
              <span>{t.restartToApply}</span>
            </span>
            <button className="primary small" onClick={() => api.restart()}>
              {t.restartNow}
            </button>
          </div>
        )}
      </div>
      {error && <p className="hint error page-note">{error}</p>}

      <h2 className="group-label">{t.byHandTitle}</h2>
      <div className="group">
        <button className="row-item" onClick={onExport}>
          <span className="tile">
            <ExportIcon size={20} />
          </span>
          <span className="row-text">
            <b>{t.exportRow}</b>
            <span>{t.exportRowHint}</span>
          </span>
          <CaretRightIcon className="go" size={16} weight="bold" />
        </button>
        <button className="row-item" onClick={onImport}>
          <span className="tile">
            <DownloadSimpleIcon size={20} />
          </span>
          <span className="row-text">
            <b>{t.importRow}</b>
            <span>{t.importRowHint}</span>
          </span>
          <CaretRightIcon className="go" size={16} weight="bold" />
        </button>
        <button className="row-item" onClick={onConnect}>
          <span className="tile">
            <LinkSimpleIcon size={20} />
          </span>
          <span className="row-text">
            <b>{t.connectRow}</b>
            <span>{t.connectRowHint}</span>
          </span>
          <CaretRightIcon className="go" size={16} weight="bold" />
        </button>
      </div>

      {status && (
        <>
          <h2 className="group-label">{t.queueTitle}</h2>
          <p className="page-note">
            {t.waitingToSend(status.outbox)} · {t.carrying(status.carried)}
          </p>
        </>
      )}
      {preview && <p className="page-note preview-tag">{t.preview}</p>}
    </div>
  );
}

import {
  BluetoothIcon,
  CaretRightIcon,
  CellTowerIcon,
  DevicesIcon,
  DownloadSimpleIcon,
  EarSlashIcon,
  ExportIcon,
  GlobeSimpleIcon,
  GraphIcon,
  LinkSimpleIcon,
  ShareNetworkIcon,
  ShieldCheckIcon,
  SpeakerHighIcon,
  WifiHighIcon,
} from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { api, type LoraStatus, type ReticulumStatus, type Status, type WifiAware } from "./api";
import { useI18n } from "./i18n";
import { bluetoothLabel, internetLabel, lanLabel, linkStates, loraLabel, reticulumLabel, wifiAwareLabel } from "./links";
import { LoraSheet } from "./LoraSheet";
import { setSoundInaudible, soundInaudible } from "./prefs";
import { ReticulumSheet } from "./ReticulumSheet";
import { canListen, listenForMessages, useMessageListening } from "./sound";

/** Bluetooth as the node started with it: a change shows the restart row until the app restarts. */
let bluetoothAtStart: boolean | null = null;

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
  // Tor and Bluetooth as now chosen. Whether the node uses Tor shows in its
  // status; Bluetooth as it started with is kept for as long as the page runs.
  const [tor, setTor] = useState<boolean | null>(null);
  const [bluetoothOn, setBluetoothOn] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Only phones have Wi-Fi Aware and Wi-Fi Direct; they come and go with Wi-Fi.
  const [wifiAware, setWifiAware] = useState<WifiAware>(null);
  const [wifiDirect, setWifiDirect] = useState<WifiAware>(null);
  const [lora, setLora] = useState<LoraStatus | null>(null);
  const [loraOpen, setLoraOpen] = useState(false);
  const [reticulum, setReticulum] = useState<ReticulumStatus | null>(null);
  const [reticulumOpen, setReticulumOpen] = useState(false);
  const soundListening = useMessageListening();
  const [inaudible, setInaudible] = useState(soundInaudible);

  useEffect(() => {
    api.wifiAware().then(setWifiAware);
    api.wifiDirect().then(setWifiDirect);
    api.lora().then(setLora);
    api.reticulum().then(setReticulum);
  }, [status, loraOpen, reticulumOpen]);

  useEffect(() => {
    api.settings().then((settings) => {
      bluetoothAtStart ??= settings.bluetooth;
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

  // The switches wait for the settings to load.
  const loaded = tor !== null;
  const torChanged = status !== null && status.internetOn && tor !== null && tor !== (status.internetTor !== null);
  const needsRestart = torChanged || (bluetoothAtStart !== null && bluetoothOn !== bluetoothAtStart);
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
            disabled={!loaded}
            onChange={(event) => toggleBluetooth(event.target.checked)}
            aria-label={t.bluetoothSetting}
          />
        </label>
        {wifiAware !== null && (
          <div className="row-item" title={t.wifiAwareHint}>
            <span className="tile">
              <DevicesIcon size={20} />
            </span>
            <span className="row-text">
              <b>{t.wifiAwareName}</b>
              <span className={(status?.wifiAwarePeers ?? 0) > 0 ? "live" : undefined}>
                {wifiAwareLabel(wifiAware, status, t)}
              </span>
            </span>
          </div>
        )}
        {wifiDirect !== null && (
          <div className="row-item" title={t.wifiDirectHint}>
            <span className="tile">
              <ShareNetworkIcon size={20} />
            </span>
            <span className="row-text">
              <b>{t.wifiDirectName}</b>
              <span className={(status?.wifiDirectPeers ?? 0) > 0 ? "live" : undefined}>
                {wifiAwareLabel(wifiDirect, status, t, status?.wifiDirectPeers ?? 0)}
              </span>
            </span>
          </div>
        )}
        <button className="row-item" title={t.loraHint} onClick={() => setLoraOpen(true)}>
          <span className="tile">
            <CellTowerIcon size={20} />
          </span>
          <span className="row-text">
            <b>{t.loraName}</b>
            <span className={lora?.region === "UNSET" ? "trouble" : lora?.connected ? "live" : undefined}>
              {loraLabel(lora, t)}
            </span>
          </span>
          <CaretRightIcon className="go" size={16} weight="bold" />
        </button>
        {canListen() && (
          <label className="row-item toggle" title={t.soundHint}>
            <span className="tile">
              <SpeakerHighIcon size={20} />
            </span>
            <span className="row-text">
              <b>{t.soundName}</b>
              <span className={soundListening === "on" ? "live" : soundListening === "failed" ? "trouble" : undefined}>
                {soundListening === "on"
                  ? t.soundListeningRow
                  : soundListening === "failed"
                    ? t.soundNoMicrophone
                    : t.stateOff}
              </span>
            </span>
            <input
              type="checkbox"
              checked={soundListening !== "off"}
              onChange={(event) => listenForMessages(event.target.checked)}
              aria-label={t.soundListenSetting}
            />
          </label>
        )}
        <label className="row-item toggle" title={t.soundInaudibleHint}>
          <span className="tile">
            <EarSlashIcon size={20} />
          </span>
          <span className="row-text">
            <b>{t.soundInaudibleName}</b>
            <span>{inaudible ? t.soundInaudibleRow : t.stateOff}</span>
          </span>
          <input
            type="checkbox"
            checked={inaudible}
            onChange={(event) => {
              setInaudible(event.target.checked);
              setSoundInaudible(event.target.checked);
            }}
            aria-label={t.soundInaudibleSetting}
          />
        </label>
        <button className="row-item" title={t.reticulumHint} onClick={() => setReticulumOpen(true)}>
          <span className="tile">
            <GraphIcon size={20} />
          </span>
          <span className="row-text">
            <b>{t.reticulumName}</b>
            <span className={reticulum?.connected ? "live" : undefined}>{reticulumLabel(reticulum, t)}</span>
          </span>
          <CaretRightIcon className="go" size={16} weight="bold" />
        </button>
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
            checked={tor ?? false}
            disabled={!loaded}
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
      {loraOpen && <LoraSheet onClose={() => setLoraOpen(false)} />}
      {reticulumOpen && <ReticulumSheet onClose={() => setReticulumOpen(false)} />}
    </div>
  );
}

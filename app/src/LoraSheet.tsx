import { useEffect, useState, type FormEvent } from "react";
import { BluetoothIcon, CaretRightIcon, CellTowerIcon, UsbIcon } from "@phosphor-icons/react";
import { api, type FoundDevice, type LoraPort, type LoraStatus } from "./api";
import { useI18n } from "./i18n";
import { Sheet } from "./Sheet";

/** How long a Bluetooth search runs on the phone. */
const SEARCH_MS = 10_000;

/** Choosing a Meshtastic device for LoRa, and what it hears once chosen. */
export function LoraSheet({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const [lora, setLora] = useState<LoraStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const refresh = () => api.lora().then(setLora);
    refresh();
    const timer = window.setInterval(refresh, 2000);
    return () => window.clearInterval(timer);
  }, []);

  async function choose(device: string | null) {
    setError(null);
    try {
      await api.setLora(device);
      setLora(await api.lora());
    } catch (failure) {
      setError(String(failure).replace(/^Error: /, ""));
    }
  }

  return (
    <Sheet title={t.loraName} onClose={onClose}>
      {lora?.device ? <Chosen lora={lora} onForget={() => choose(null)} /> : <Choose onChoose={choose} />}
      {error && <p className="hint error">{error}</p>}
    </Sheet>
  );
}

function Chosen({ lora, onForget }: { lora: LoraStatus; onForget: () => void }) {
  const { t } = useI18n();
  const now = Math.floor(Date.now() / 1000);
  const ago = (at: number) => t.duration(Math.max(0, now - at));
  const unset = lora.region === "UNSET";
  return (
    <>
      <div className="group">
        <div className="row-item">
          <span className="tile">
            <CellTowerIcon size={20} />
          </span>
          <span className="row-text">
            <b>{lora.name ?? t.loraDevice}</b>
            <span className={lora.connected ? "live" : undefined}>
              {lora.connected ? t.loraConnected : t.stateConnecting}
            </span>
          </span>
        </div>
        {lora.region && (
          <div className="row-item">
            <span className="row-text">
              <b>{t.loraRegion}</b>
              <span className={unset ? "trouble" : undefined}>{lora.region}</span>
            </span>
          </div>
        )}
        {lora.waiting > 0 && (
          <div className="row-item">
            <span className="row-text">
              <b>{t.loraWaiting}</b>
              <span>{lora.waiting}</span>
            </span>
          </div>
        )}
      </div>
      {unset && <p className="hint error">{t.loraRegionHint}</p>}
      <p className="hint">{lora.carrychatAt === null ? t.loraCarryChatNever : t.loraCarryChatAt(ago(lora.carrychatAt))}</p>
      <h2 className="group-label">{t.loraHeardTitle}</h2>
      {lora.heard.length === 0 ? (
        <p className="hint">{t.loraNoneHeard}</p>
      ) : (
        <div className="group">
          {lora.heard.map((node) => (
            <div className="row-item" key={node.name + node.at}>
              <span className="row-text">
                <b>{node.name}</b>
                <span>{t.loraNodeLine(ago(node.at), node.snr, node.hops)}</span>
              </span>
            </div>
          ))}
        </div>
      )}
      <p className="hint mono">{lora.device}</p>
      <div className="dialog-actions">
        <button className="danger-outline" onClick={onForget}>
          {t.loraForget}
        </button>
      </div>
    </>
  );
}

function Choose({ onChoose }: { onChoose: (device: string) => void }) {
  const { t } = useI18n();
  // Phones reach Meshtastic devices over Bluetooth; computers over USB or the network.
  const phone = Boolean(window.CarryChatAndroid);
  return (
    <>
      <p>{t.loraIntro}</p>
      {phone ? <Search onChoose={onChoose} /> : <Ports onChoose={onChoose} />}
      <Address onChoose={onChoose} />
    </>
  );
}

function Search({ onChoose }: { onChoose: (device: string) => void }) {
  const { t } = useI18n();
  const [found, setFound] = useState<FoundDevice[]>([]);
  const [searching, setSearching] = useState(false);
  const [searched, setSearched] = useState(false);

  async function search() {
    setSearching(true);
    setFound([]);
    await api.loraScan();
    const timer = window.setInterval(() => api.loraFound().then(setFound), 1000);
    window.setTimeout(() => {
      window.clearInterval(timer);
      api.loraFound().then(setFound);
      setSearching(false);
      setSearched(true);
    }, SEARCH_MS);
  }

  return (
    <>
      <div className="group">
        {found.map((device) => (
          <button key={device.address} className="row-item" onClick={() => onChoose(`bluetooth:${device.address}`)}>
            <span className="tile">
              <BluetoothIcon size={20} />
            </span>
            <span className="row-text">
              <b>{device.name ?? device.address}</b>
              <span className="mono">{device.address}</span>
            </span>
            <CaretRightIcon className="go" size={16} weight="bold" />
          </button>
        ))}
        {searched && !searching && found.length === 0 && (
          <div className="row-item">
            <span className="row-text">
              <span>{t.loraNoneFound}</span>
            </span>
          </div>
        )}
      </div>
      <p className="hint">{t.loraPairHint}</p>
      <div className="dialog-actions">
        <button className="primary" onClick={search} disabled={searching}>
          {searching ? t.loraSearching : t.loraSearch}
        </button>
      </div>
    </>
  );
}

function Ports({ onChoose }: { onChoose: (device: string) => void }) {
  const { t } = useI18n();
  const [ports, setPorts] = useState<LoraPort[] | null>(null);
  const refresh = () => api.loraPorts().then(setPorts);

  useEffect(() => {
    refresh();
  }, []);

  return (
    <>
      <h2 className="group-label">{t.loraPorts}</h2>
      <div className="group">
        {ports?.map((port) => (
          <button key={port.path} className="row-item" onClick={() => onChoose(port.path)}>
            <span className="tile">
              <UsbIcon size={20} />
            </span>
            <span className="row-text">
              <b>{port.label ?? port.path}</b>
              <span className="mono">{port.path}</span>
            </span>
            <CaretRightIcon className="go" size={16} weight="bold" />
          </button>
        ))}
        {ports?.length === 0 && (
          <div className="row-item">
            <span className="row-text">
              <span>{t.loraNoPorts}</span>
            </span>
            <button className="small" onClick={refresh}>
              {t.loraRefresh}
            </button>
          </div>
        )}
      </div>
    </>
  );
}

function Address({ onChoose }: { onChoose: (device: string) => void }) {
  const { t } = useI18n();
  const [address, setAddress] = useState("");

  function submit(event: FormEvent) {
    event.preventDefault();
    onChoose(`tcp:${address.trim()}`);
  }

  return (
    <form onSubmit={submit}>
      <label htmlFor="lora-address">{t.loraOnNetwork}</label>
      <input
        id="lora-address"
        className="mono"
        value={address}
        onChange={(event) => setAddress(event.target.value)}
        placeholder="192.168.1.20:4403"
      />
      <div className="dialog-actions">
        <button className="primary" disabled={address.trim() === ""}>
          {t.loraConnect}
        </button>
      </div>
    </form>
  );
}

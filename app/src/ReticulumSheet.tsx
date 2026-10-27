import { useEffect, useState, type FormEvent } from "react";
import { GraphIcon } from "@phosphor-icons/react";
import { api, type ReticulumStatus } from "./api";
import { useI18n } from "./i18n";
import { Sheet } from "./Sheet";

/** Choosing a Reticulum node to connect to, and the CarryChat nodes heard through it. */
export function ReticulumSheet({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const [reticulum, setReticulum] = useState<ReticulumStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const refresh = () => api.reticulum().then(setReticulum);
    refresh();
    const timer = window.setInterval(refresh, 2000);
    return () => window.clearInterval(timer);
  }, []);

  async function choose(address: string | null) {
    setError(null);
    try {
      await api.setReticulum(address);
      setReticulum(await api.reticulum());
    } catch (failure) {
      setError(String(failure).replace(/^Error: /, ""));
    }
  }

  return (
    <Sheet title={t.reticulumName} onClose={onClose}>
      {reticulum?.address ? (
        <Connected reticulum={reticulum} onForget={() => choose(null)} />
      ) : (
        <Address onChoose={choose} />
      )}
      {error && <p className="hint error">{error}</p>}
    </Sheet>
  );
}

function Connected({ reticulum, onForget }: { reticulum: ReticulumStatus; onForget: () => void }) {
  const { t } = useI18n();
  const now = Math.floor(Date.now() / 1000);
  const ago = (at: number) => t.duration(Math.max(0, now - at));
  return (
    <>
      <div className="group">
        <div className="row-item">
          <span className="tile">
            <GraphIcon size={20} />
          </span>
          <span className="row-text">
            <b className="mono">{reticulum.address}</b>
            <span className={reticulum.connected ? "live" : undefined}>
              {reticulum.connected ? t.loraConnected : t.stateConnecting}
            </span>
          </span>
        </div>
        {reticulum.waiting > 0 && (
          <div className="row-item">
            <span className="row-text">
              <b>{t.reticulumWaiting}</b>
              <span>{reticulum.waiting}</span>
            </span>
          </div>
        )}
      </div>
      <p className="hint">
        {reticulum.carrychatAt === null ? t.reticulumCarryChatNever : t.reticulumCarryChatAt(ago(reticulum.carrychatAt))}
      </p>
      <h2 className="group-label">{t.reticulumPeersTitle}</h2>
      {reticulum.peers.length === 0 ? (
        <p className="hint">{t.reticulumNoPeers}</p>
      ) : (
        <div className="group">
          {reticulum.peers.map((peer) => (
            <div className="row-item" key={peer.destination}>
              <span className="row-text">
                <b className="mono">{peer.destination.slice(0, 16)}</b>
                <span>{t.reticulumPeerLine(ago(peer.at), peer.hops)}</span>
              </span>
            </div>
          ))}
        </div>
      )}
      {reticulum.destination && (
        <p className="hint">
          {t.reticulumOwn} <span className="mono">{reticulum.destination}</span>
        </p>
      )}
      <div className="dialog-actions">
        <button className="danger-outline" onClick={onForget}>
          {t.reticulumForget}
        </button>
      </div>
    </>
  );
}

function Address({ onChoose }: { onChoose: (address: string) => void }) {
  const { t } = useI18n();
  const [address, setAddress] = useState("");

  function submit(event: FormEvent) {
    event.preventDefault();
    onChoose(address.trim());
  }

  return (
    <form onSubmit={submit}>
      <p>{t.reticulumIntro}</p>
      <label htmlFor="reticulum-address">{t.reticulumAddress}</label>
      <input
        id="reticulum-address"
        className="mono"
        value={address}
        onChange={(event) => setAddress(event.target.value)}
        placeholder="127.0.0.1:4242"
      />
      <p className="hint">{t.reticulumAddressHint}</p>
      <div className="dialog-actions">
        <button className="primary" disabled={address.trim() === ""}>
          {t.loraConnect}
        </button>
      </div>
    </form>
  );
}

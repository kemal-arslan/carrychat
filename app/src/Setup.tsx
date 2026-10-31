import { useState, type FormEvent } from "react";
import { api, type Profile } from "./api";
import { MAX_NAME_BYTES, utf8Length } from "./format";
import { useI18n } from "./i18n";
import { Logo } from "./Logo";

/**
 * Shown when the data on this device does not open, as when the key store holding its key refused or could not be
 * reached. The data stays as it is: trying again opens it once the key can be read.
 */
export function Locked({ reason, onOpened }: { reason: string; onOpened: (profile: Profile | null) => void }) {
  const { t } = useI18n();
  const [error, setError] = useState(reason);
  const [busy, setBusy] = useState(false);

  async function retry(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    try {
      onOpened(await api.unlock());
    } catch (failure) {
      setError(String(failure).replace(/^Error: /, ""));
      setBusy(false);
    }
  }

  return (
    <div className="setup">
      <form className="setup-card" onSubmit={retry}>
        <Logo size={64} />
        <h1>{t.lockedTitle}</h1>
        <p className="lede">{t.lockedIntro}</p>
        <p className="hint error">{error}</p>
        <button className="primary" disabled={busy} autoFocus>
          {t.lockedRetry}
        </button>
      </form>
    </div>
  );
}

export function Setup({ onCreated }: { onCreated: (profile: Profile) => void }) {
  const { t } = useI18n();
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const tooLong = utf8Length(name.trim()) > MAX_NAME_BYTES;

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      onCreated(await api.createProfile(name));
    } catch (failure) {
      setError(String(failure).replace(/^Error: /, ""));
      setBusy(false);
    }
  }

  return (
    <div className="setup">
      <form className="setup-card" onSubmit={submit}>
        <Logo size={64} />
        <h1>CarryChat</h1>
        <p className="lede">{t.tagline}</p>
        <label htmlFor="name">{t.yourName}</label>
        <input
          id="name"
          autoFocus
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder="Ayşe Yılmaz"
          aria-invalid={tooLong}
        />
        <p className={tooLong ? "hint error" : "hint"}>{tooLong ? t.nameTooLong : t.nameHint}</p>
        {error && <p className="hint error">{error}</p>}
        <button className="primary" disabled={busy || tooLong || name.trim() === ""}>
          {t.createIdentity}
        </button>
      </form>
    </div>
  );
}

import { useState, type FormEvent } from "react";
import { api, type Profile } from "./api";
import { MAX_NAME_BYTES, utf8Length } from "./format";
import { useI18n } from "./i18n";
import { Logo } from "./Logo";

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

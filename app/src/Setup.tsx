import { useState, type FormEvent } from "react";
import { api, type Profile } from "./api";
import { Logo } from "./Logo";
import { MAX_NAME_BYTES, utf8Length } from "./format";

export function Setup({ onCreated }: { onCreated: (profile: Profile) => void }) {
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
      setError(String(failure));
      setBusy(false);
    }
  }

  return (
    <div className="setup">
      <form className="setup-card" onSubmit={submit}>
        <Logo size={64} />
        <h1>Kervan</h1>
        <p className="lede">Messages that get through when the internet doesn't.</p>
        <label htmlFor="name">Your name</label>
        <input
          id="name"
          autoFocus
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder="Ayşe Yılmaz"
          aria-invalid={tooLong}
        />
        <p className={tooLong ? "hint error" : "hint"}>
          {tooLong
            ? "That name is too long for a contact card."
            : "Your contacts see this name. There is no account: your identity is created and kept on this computer."}
        </p>
        {error && <p className="hint error">{error}</p>}
        <button className="primary" disabled={busy || tooLong || name.trim() === ""}>
          Create my identity
        </button>
      </form>
    </div>
  );
}

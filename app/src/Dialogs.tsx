import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { api, type Contact, type Profile } from "./api";

function Dialog({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  return (
    <dialog ref={ref} onClose={onClose} onCancel={onClose} aria-label={title}>
      <h2>{title}</h2>
      {children}
    </dialog>
  );
}

export function AddContactDialog({ onClose, onAdded }: { onClose: () => void; onAdded: (contact: Contact) => void }) {
  const [card, setCard] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [added, setAdded] = useState<Contact | null>(null);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      const contact = await api.addContact(card);
      setAdded(contact);
      onAdded(contact);
    } catch (failure) {
      setError(String(failure).replace(/^Error: /, ""));
    }
  }

  if (added) {
    return (
      <Dialog title={`${added.name} is a contact`} onClose={onClose}>
        <p>Before you rely on it, check that {added.name} sees this fingerprint under My card:</p>
        <p className="fingerprint large">{added.fingerprint}</p>
        <p className="hint">If it differs, the card was changed on its way to you.</p>
        <div className="dialog-actions">
          <button className="primary" onClick={onClose} autoFocus>
            Done
          </button>
        </div>
      </Dialog>
    );
  }

  return (
    <Dialog title="Add a contact" onClose={onClose}>
      <form onSubmit={submit}>
        <label htmlFor="card">Their card</label>
        <textarea
          id="card"
          className="mono"
          rows={5}
          value={card}
          onChange={(event) => setCard(event.target.value)}
          placeholder="kervan:…"
          autoFocus
        />
        <p className={error ? "hint error" : "hint"}>
          {error ?? "Paste the card they gave you. It starts with kervan:"}
        </p>
        <div className="dialog-actions">
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button className="primary" disabled={card.trim() === ""}>
            Add
          </button>
        </div>
      </form>
    </Dialog>
  );
}

export function MyCardDialog({ profile, onClose }: { profile: Profile; onClose: () => void }) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    await navigator.clipboard.writeText(profile.card);
    setCopied(true);
  }

  return (
    <Dialog title="My card" onClose={onClose}>
      <p>Give this card to the people you want to reach, and add theirs.</p>
      <textarea className="mono" rows={5} readOnly value={profile.card} onFocus={(event) => event.target.select()} />
      <p className="hint">Your fingerprint. Read it out to your contacts so they can check your card:</p>
      <p className="fingerprint large">{profile.fingerprint}</p>
      <div className="dialog-actions">
        <button onClick={onClose}>Close</button>
        <button className="primary" onClick={copy}>
          {copied ? "Copied" : "Copy card"}
        </button>
      </div>
    </Dialog>
  );
}

export function ConnectDialog({
  port,
  onClose,
  onConnected,
}: {
  port: number | null;
  onClose: () => void;
  onConnected: (notice: string) => void;
}) {
  const [address, setAddress] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.connectPeer(address);
      onConnected(`Connected to ${address.trim()}.`);
      onClose();
    } catch (failure) {
      setError(String(failure).replace(/^Error: /, ""));
      setBusy(false);
    }
  }

  return (
    <Dialog title="Connect to a computer" onClose={onClose}>
      <form onSubmit={submit}>
        <p>
          Computers on the same network usually find each other. If they don't, connect by address.
          {port !== null && (
            <>
              {" "}
              This computer listens on port <strong>{port}</strong>.
            </>
          )}
        </p>
        <label htmlFor="address">Address</label>
        <input
          id="address"
          className="mono"
          value={address}
          onChange={(event) => setAddress(event.target.value)}
          placeholder="192.168.1.20:49333"
          autoFocus
        />
        {error && <p className="hint error">{error}</p>}
        <div className="dialog-actions">
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button className="primary" disabled={busy || address.trim() === ""}>
            Connect
          </button>
        </div>
      </form>
    </Dialog>
  );
}

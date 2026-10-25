import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { api, type Contact, type LanProblem, type Profile } from "./api";

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
          {error ?? "Paste the card they gave you. It starts with kervan:. They don't need yours: your first message carries it."}
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
      <p>
        Give this card to the people you want to hear from. They can write to you right away, and you see their first
        message as a request.
      </p>
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

export function NetworkDialog({ problem, onClose }: { problem: LanProblem; onClose: () => void }) {
  const [error, setError] = useState<string | null>(null);

  async function openSettings() {
    try {
      await api.openNetworkSettings();
    } catch (failure) {
      setError(String(failure).replace(/^Error: /, ""));
    }
  }

  if (problem.kind === "failed") {
    return (
      <Dialog title="The local network is not working" onClose={onClose}>
        <p>Kervan could not reach another computer on this network:</p>
        <p className="mono problem-detail">{problem.detail}</p>
        <p className="hint">
          Check that both computers are on the same network and that the other computer's firewall lets Kervan
          accept connections. Guest networks often keep computers from reaching each other.
        </p>
        <div className="dialog-actions">
          <button className="primary" onClick={onClose} autoFocus>
            Close
          </button>
        </div>
      </Dialog>
    );
  }

  return (
    <Dialog title="Let Kervan use the local network" onClose={onClose}>
      <p>macOS is not letting Kervan reach other computers on this network, so messages cannot leave this Mac.</p>
      <ol className="steps">
        <li>
          Open <strong>System Settings → Privacy &amp; Security → Local Network</strong>.
        </li>
        <li>
          Turn on <strong>Kervan</strong>.
        </li>
        <li>If this still shows as blocked after a few seconds, quit Kervan and open it again.</li>
      </ol>
      <p className="mono problem-detail">{problem.detail}</p>
      {error && <p className="hint error">{error}</p>}
      <div className="dialog-actions">
        <button onClick={onClose}>Close</button>
        <button className="primary" onClick={openSettings} autoFocus>
          Open System Settings
        </button>
      </div>
    </Dialog>
  );
}

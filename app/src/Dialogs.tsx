import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import QRCode from "qrcode";
import { api, type Contact, type Group, type LanProblem, type Member, type Profile } from "./api";
import { MAX_NAME_BYTES, utf8Length } from "./format";
import { useI18n, type LanguageChoice } from "./i18n";
import { notificationsEnabled, setNotificationsEnabled } from "./prefs";

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

function errorText(failure: unknown): string {
  return String(failure).replace(/^Error: /, "");
}

export function AddContactDialog({ onClose, onAdded }: { onClose: () => void; onAdded: (contact: Contact) => void }) {
  const { t } = useI18n();
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
      setError(errorText(failure));
    }
  }

  if (added) {
    return (
      <Dialog title={t.addedTitle(added.name)} onClose={onClose}>
        <p>{t.addedCheck(added.name)}</p>
        <p className="fingerprint large">{added.fingerprint}</p>
        <p className="hint">{t.addedWarning}</p>
        <div className="dialog-actions">
          <button className="primary" onClick={onClose} autoFocus>
            {t.done}
          </button>
        </div>
      </Dialog>
    );
  }

  return (
    <Dialog title={t.addTitle} onClose={onClose}>
      <form onSubmit={submit}>
        <label htmlFor="card">{t.theirCard}</label>
        <textarea
          id="card"
          className="mono"
          rows={5}
          value={card}
          onChange={(event) => setCard(event.target.value)}
          placeholder="kervan:…"
          autoFocus
        />
        <p className={error ? "hint error" : "hint"}>{error ?? t.addHint}</p>
        <div className="dialog-actions">
          <button type="button" onClick={onClose}>
            {t.cancel}
          </button>
          <button className="primary" disabled={card.trim() === ""}>
            {t.add}
          </button>
        </div>
      </form>
    </Dialog>
  );
}

export function MyCardDialog({ profile, onClose }: { profile: Profile; onClose: () => void }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  const [qr, setQr] = useState<string | null>(null);

  useEffect(() => {
    QRCode.toString(profile.card, { type: "svg", margin: 1, errorCorrectionLevel: "M" }).then((svg) =>
      setQr(`data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`),
    );
  }, [profile.card]);

  async function copy() {
    await navigator.clipboard.writeText(profile.card);
    setCopied(true);
  }

  return (
    <Dialog title={t.myCard} onClose={onClose}>
      <p>{t.cardIntro}</p>
      {qr && <img className="qr" src={qr} alt={profile.card} width={200} height={200} />}
      <p className="hint center">{t.cardQr}</p>
      <textarea className="mono" rows={4} readOnly value={profile.card} onFocus={(event) => event.target.select()} />
      <p className="hint">{t.cardFingerprint}</p>
      <p className="fingerprint large">{profile.fingerprint}</p>
      <div className="dialog-actions">
        <button onClick={onClose}>{t.close}</button>
        <button className="primary" onClick={copy}>
          {copied ? t.copied : t.copyCard}
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
  const { t } = useI18n();
  const [address, setAddress] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.connectPeer(address);
      onConnected(t.connected(address.trim()));
      onClose();
    } catch (failure) {
      setError(errorText(failure));
      setBusy(false);
    }
  }

  return (
    <Dialog title={t.connectTitle} onClose={onClose}>
      <form onSubmit={submit}>
        <p>
          {t.connectIntro}
          {port !== null && t.listensOn(port)}
        </p>
        <label htmlFor="address">{t.address}</label>
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
            {t.cancel}
          </button>
          <button className="primary" disabled={busy || address.trim() === ""}>
            {t.connectButton}
          </button>
        </div>
      </form>
    </Dialog>
  );
}

export function NetworkDialog({ problem, onClose }: { problem: LanProblem; onClose: () => void }) {
  const { t } = useI18n();
  const [error, setError] = useState<string | null>(null);

  async function openSettings() {
    try {
      await api.openNetworkSettings();
    } catch (failure) {
      setError(errorText(failure));
    }
  }

  if (problem.kind === "failed") {
    return (
      <Dialog title={t.failedTitle} onClose={onClose}>
        <p>{t.failedIntro}</p>
        <p className="mono problem-detail">{problem.detail}</p>
        <p className="hint">{t.failedHint}</p>
        <div className="dialog-actions">
          <button className="primary" onClick={onClose} autoFocus>
            {t.close}
          </button>
        </div>
      </Dialog>
    );
  }

  return (
    <Dialog title={t.blockedTitle} onClose={onClose}>
      <p>{t.blockedIntro}</p>
      <ol className="steps">
        <li>{t.blockedStep1}</li>
        <li>{t.blockedStep2}</li>
        <li>{t.blockedStep3}</li>
      </ol>
      <p className="mono problem-detail">{problem.detail}</p>
      {error && <p className="hint error">{error}</p>}
      <div className="dialog-actions">
        <button onClick={onClose}>{t.close}</button>
        <button className="primary" onClick={openSettings} autoFocus>
          {t.openSystemSettings}
        </button>
      </div>
    </Dialog>
  );
}

export function SettingsDialog({
  profile,
  onClose,
  onRenamed,
}: {
  profile: Profile;
  onClose: () => void;
  onRenamed: (profile: Profile) => void;
}) {
  const { t, choice, setChoice } = useI18n();
  const [name, setName] = useState(profile.name);
  const [readReceipts, setReadReceipts] = useState<boolean | null>(null);
  const [notifications, setNotifications] = useState(notificationsEnabled);
  const [confirmErase, setConfirmErase] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const trimmed = name.trim();
  const tooLong = utf8Length(trimmed) > MAX_NAME_BYTES;

  useEffect(() => {
    api.settings().then((settings) => setReadReceipts(settings.readReceipts));
  }, []);

  async function rename(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      onRenamed(await api.rename(trimmed));
    } catch (failure) {
      setError(errorText(failure));
    }
  }

  async function toggleReadReceipts(on: boolean) {
    setReadReceipts(on);
    await api.setReadReceipts(on).catch((failure) => setError(errorText(failure)));
  }

  function toggleNotifications(on: boolean) {
    setNotifications(on);
    setNotificationsEnabled(on);
  }

  async function erase() {
    try {
      await api.eraseEverything();
    } catch (failure) {
      setError(errorText(failure));
    }
  }

  return (
    <Dialog title={t.settingsTitle} onClose={onClose}>
      <form onSubmit={rename}>
        <label htmlFor="rename">{t.yourName}</label>
        <div className="inline-field">
          <input id="rename" value={name} onChange={(event) => setName(event.target.value)} aria-invalid={tooLong} />
          <button className="primary" disabled={tooLong || trimmed === "" || trimmed === profile.name}>
            {t.save}
          </button>
        </div>
        <p className={tooLong ? "hint error" : "hint"}>{tooLong ? t.nameTooLong : t.renameHint}</p>
      </form>

      <h3 className="settings-section">{t.privacy}</h3>
      <label className="toggle">
        <input
          type="checkbox"
          checked={readReceipts ?? true}
          disabled={readReceipts === null}
          onChange={(event) => toggleReadReceipts(event.target.checked)}
        />
        {t.readReceipts}
      </label>
      <label className="toggle">
        <input
          type="checkbox"
          checked={notifications}
          onChange={(event) => toggleNotifications(event.target.checked)}
        />
        {t.notifications}
      </label>

      <label htmlFor="language">{t.language}</label>
      <select id="language" value={choice} onChange={(event) => setChoice(event.target.value as LanguageChoice)}>
        <option value="system">{t.languageSystem}</option>
        <option value="en">English</option>
        <option value="tr">Türkçe</option>
      </select>

      <h3 className="settings-section danger">{t.eraseTitle}</h3>
      <p className="hint">{confirmErase ? t.eraseConfirm : t.eraseHint}</p>
      {error && <p className="hint error">{error}</p>}
      <div className="dialog-actions">
        {confirmErase ? (
          <>
            <button onClick={() => setConfirmErase(false)}>{t.cancel}</button>
            <button className="danger" onClick={erase}>
              {t.eraseNow}
            </button>
          </>
        ) : (
          <>
            <button className="danger-outline" onClick={() => setConfirmErase(true)}>
              {t.eraseButton}
            </button>
            <button className="primary" onClick={onClose}>
              {t.close}
            </button>
          </>
        )}
      </div>
    </Dialog>
  );
}

export function NewGroupDialog({
  contacts,
  onClose,
  onCreated,
}: {
  contacts: Contact[];
  onClose: () => void;
  onCreated: (group: Group) => void;
}) {
  const { t } = useI18n();
  const [name, setName] = useState("");
  const [chosen, setChosen] = useState<number[]>([]);
  const [error, setError] = useState<string | null>(null);
  const candidates = contacts.filter((contact) => !contact.request);
  const tooLong = utf8Length(name.trim()) > MAX_NAME_BYTES;

  function toggle(id: number) {
    setChosen((current) => (current.includes(id) ? current.filter((other) => other !== id) : [...current, id]));
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      onCreated(await api.createGroup(name, chosen));
      onClose();
    } catch (failure) {
      setError(errorText(failure));
    }
  }

  return (
    <Dialog title={t.newGroup} onClose={onClose}>
      <form onSubmit={submit}>
        <label htmlFor="group-name">{t.groupName}</label>
        <input id="group-name" value={name} onChange={(event) => setName(event.target.value)} autoFocus />
        <h3 className="settings-section">{t.groupWho}</h3>
        {candidates.length === 0 ? (
          <p className="hint">{t.groupNoContacts}</p>
        ) : (
          <ul className="pick-list">
            {candidates.map((contact) => (
              <li key={contact.id}>
                <label className={contact.hasOurCard ? undefined : "disabled"}>
                  <input
                    type="checkbox"
                    checked={chosen.includes(contact.id)}
                    disabled={!contact.hasOurCard}
                    onChange={() => toggle(contact.id)}
                  />
                  <span>{contact.name}</span>
                  {!contact.hasOurCard && <span className="hint">{t.needsYourCard}</span>}
                </label>
              </li>
            ))}
          </ul>
        )}
        <p className={error ? "hint error" : "hint"}>{error ?? (tooLong ? t.groupNameTooLong : t.groupHint)}</p>
        <div className="dialog-actions">
          <button type="button" onClick={onClose}>
            {t.cancel}
          </button>
          <button className="primary" disabled={name.trim() === "" || tooLong || chosen.length === 0}>
            {t.create}
          </button>
        </div>
      </form>
    </Dialog>
  );
}

export function MembersDialog({
  group,
  contacts,
  onClose,
  onChanged,
  onMessage,
}: {
  group: Group;
  contacts: Contact[];
  onClose: () => void;
  onChanged: (group: Group) => void;
  onMessage: (member: Member) => Promise<void>;
}) {
  const { t } = useI18n();
  const [adding, setAdding] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [confirmLeave, setConfirmLeave] = useState(false);
  const inGroup = new Set(group.members.map((member) => member.contactId));
  const addable = contacts.filter((contact) => !contact.request && contact.hasOurCard && !inGroup.has(contact.id));

  async function change(action: () => Promise<Group>) {
    setError(null);
    try {
      onChanged(await action());
    } catch (failure) {
      setError(errorText(failure));
    }
  }

  return (
    <Dialog title={t.membersTitle(group.name)} onClose={onClose}>
      <ul className="member-list">
        {group.members.map((member) => (
          <li key={member.fingerprint}>
            <span className="who">
              <span className="name">
                {member.name}
                {member.me && <span className="tag">{t.you}</span>}
                {member.admin && <span className="tag">{t.admin}</span>}
              </span>
              <span className="fingerprint">{member.fingerprint}</span>
            </span>
            {!member.me && (
              <button
                title={member.contactId === null ? t.messageMemberHint(member.name) : undefined}
                onClick={() => onMessage(member).catch((failure) => setError(errorText(failure)))}
              >
                {t.messageMember}
              </button>
            )}
            {group.admin && group.member && !member.me && (
              <button onClick={() => change(() => api.removeGroupMember(group.id, member.fingerprint))}>
                {t.remove}
              </button>
            )}
          </li>
        ))}
      </ul>
      {group.admin && group.member && (
        <div className="inline-field">
          <select value={adding} onChange={(event) => setAdding(event.target.value)} aria-label={t.addMember}>
            <option value="">{addable.length > 0 ? t.chooseContact : t.nobodyToAdd}</option>
            {addable.map((contact) => (
              <option key={contact.id} value={contact.id}>
                {contact.name}
              </option>
            ))}
          </select>
          <button
            disabled={adding === ""}
            onClick={() =>
              change(async () => {
                const changed = await api.addGroupMember(group.id, Number(adding));
                setAdding("");
                return changed;
              })
            }
          >
            {t.addMember}
          </button>
        </div>
      )}
      <p className={error ? "hint error" : "hint"}>
        {error ?? (group.admin ? t.membersAdminHint : group.member ? t.membersHint : t.leftGroup)}
      </p>
      <div className="dialog-actions">
        {!group.admin && group.member && (
          confirmLeave ? (
            <button className="danger" onClick={() => change(() => api.leaveGroup(group.id))}>
              {t.leaveNow}
            </button>
          ) : (
            <button className="danger-outline" onClick={() => setConfirmLeave(true)}>
              {t.leaveGroup}
            </button>
          )
        )}
        <button className="primary" onClick={onClose}>
          {t.done}
        </button>
      </div>
    </Dialog>
  );
}

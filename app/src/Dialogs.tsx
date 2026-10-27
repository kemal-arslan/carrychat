import {
  CheckIcon,
  ChecksIcon,
  HashIcon,
  HourglassMediumIcon,
  LockSimpleIcon,
  ShieldCheckIcon,
  UserPlusIcon,
  UsersThreeIcon,
} from "@phosphor-icons/react";
import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import QRCode from "qrcode";
import {
  api,
  type Channel,
  type Contact,
  type Group,
  type Journey,
  type LanProblem,
  type Member,
  type Message,
  type Muted,
  type Profile,
} from "./api";
import { MAX_NAME_BYTES, clockTime, fullTime, utf8Length } from "./format";
import { useI18n } from "./i18n";
import { Sheet } from "./Sheet";
import { canListen, listenForCard, playCard, useSound } from "./sound";
import { ViaIcon } from "./Via";

const Dialog = Sheet;

/** Whether this phone passes cards over NFC (§3.3), as the sheet opens. */
function useNfc(): boolean {
  const [nfc] = useState(() => window.CarryChatAndroid?.nfcAvailable?.() ?? false);
  return nfc;
}

function errorText(failure: unknown): string {
  return String(failure).replace(/^Error: /, "");
}

export function AddContactDialog({ onClose, onAdded }: { onClose: () => void; onAdded: (contact: Contact) => void }) {
  const { t } = useI18n();
  const [card, setCard] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [added, setAdded] = useState<Contact | null>(null);
  // Listening for a card played by sound, or reading one over NFC, and whose card was heard.
  const listening = useSound();
  const [heardName, setHeardName] = useState<string | null>(null);
  const nfc = useNfc();

  useEffect(() => {
    const android = window.CarryChatAndroid;
    if (!nfc) return;
    const onCard = async (event: Event) => {
      try {
        const heard = await api.readCard((event as CustomEvent<string>).detail);
        listening.stop();
        setCard(heard.card);
        setHeardName(heard.name);
        setError(null);
      } catch (failure) {
        setError(errorText(failure));
      }
    };
    android?.nfcRead(true);
    window.addEventListener("carrychat-nfc", onCard);
    return () => {
      android?.nfcRead(false);
      window.removeEventListener("carrychat-nfc", onCard);
    };
  }, [nfc, listening.stop]);

  async function listen() {
    if (listening.active) return listening.stop();
    setError(null);
    setHeardName(null);
    try {
      await listening.start(() =>
        listenForCard((heard) => {
          listening.stop();
          setCard(heard.card);
          setHeardName(heard.name);
        }),
      );
    } catch {
      setError(t.soundNoMicrophone);
    }
  }

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
          placeholder="carrychat:…"
          autoFocus
        />
        <p className={error ? "hint error" : "hint"}>
          {error ?? (listening.active ? t.soundListening : heardName ? t.soundHeard(heardName) : t.addHint)}
        </p>
        {nfc && !heardName && <p className="hint">{t.nfcRead}</p>}
        <div className="dialog-actions">
          <button type="button" onClick={onClose}>
            {t.cancel}
          </button>
          {canListen() && (
            <button type="button" onClick={listen}>
              {listening.active ? t.soundStop : t.soundListen}
            </button>
          )}
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
  const playing = useSound();
  const nfc = useNfc();

  // Held to a phone adding a contact, this one gives its card while the sheet is open.
  useEffect(() => {
    if (!nfc) return;
    window.CarryChatAndroid?.nfcOffer(profile.card);
    return () => window.CarryChatAndroid?.nfcOffer(null);
  }, [nfc, profile.card]);

  function play() {
    if (playing.active) return playing.stop();
    playing.start(playCard).catch(() => {});
  }

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
      {playing.active && <p className="hint">{t.soundPlaying}</p>}
      {nfc && <p className="hint">{t.nfcOffer}</p>}
      <div className="dialog-actions">
        <button onClick={onClose}>{t.close}</button>
        <button onClick={play}>{playing.active ? t.soundStop : t.soundSend}</button>
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

export function JoinChannelDialog({
  defaultNick,
  onClose,
  onJoined,
}: {
  defaultNick: string;
  onClose: () => void;
  onJoined: (channel: Channel) => void;
}) {
  const { t } = useI18n();
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [nick, setNick] = useState(defaultNick);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      onJoined(await api.joinChannel(name, password, nick));
      onClose();
    } catch (failure) {
      const text = errorText(failure);
      setError(text.includes("channel name") ? t.channelNameInvalid : text);
    }
  }

  return (
    <Dialog title={t.joinChannel} onClose={onClose}>
      <form onSubmit={submit}>
        <label htmlFor="channel-name">{t.channelName}</label>
        <input
          id="channel-name"
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder="#kadıköy"
          autoFocus
        />
        <label htmlFor="channel-password">{t.channelPassword}</label>
        <input
          id="channel-password"
          type="password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
        />
        <label htmlFor="channel-nick">{t.channelNick}</label>
        <input id="channel-nick" value={nick} onChange={(event) => setNick(event.target.value)} />
        <p className={error ? "hint error" : "hint"}>{error ?? t.joinHint}</p>
        <div className="dialog-actions">
          <button type="button" onClick={onClose}>
            {t.cancel}
          </button>
          <button className="primary" disabled={name.trim() === "" || nick.trim() === ""}>
            {t.join}
          </button>
        </div>
      </form>
    </Dialog>
  );
}

export function ChannelSettingsDialog({
  channel,
  onClose,
  onChanged,
  onUnmuted,
  onShared,
  onLeft,
}: {
  channel: Channel;
  onClose: () => void;
  onChanged: () => void;
  onUnmuted: (nick: string) => void;
  onShared: () => void;
  onLeft: () => void;
}) {
  const { t } = useI18n();
  const [nick, setNick] = useState(channel.nick);
  const [error, setError] = useState<string | null>(null);
  const [confirmLeave, setConfirmLeave] = useState(false);
  const [muted, setMuted] = useState<Muted[]>([]);

  useEffect(() => {
    api.mutedAuthors(channel.id).then(setMuted);
  }, [channel.id]);

  async function run(action: () => Promise<unknown>, after: () => void) {
    setError(null);
    try {
      await action();
      after();
    } catch (failure) {
      setError(errorText(failure));
    }
  }

  return (
    <Dialog title={`#${channel.name}`} onClose={onClose}>
      <label htmlFor="settings-nick">{t.channelNick}</label>
      <div className="inline-field">
        <input id="settings-nick" value={nick} onChange={(event) => setNick(event.target.value)} />
        <button
          disabled={nick.trim() === "" || nick.trim() === channel.nick}
          onClick={() => run(() => api.setChannelNick(channel.id, nick), onChanged)}
        >
          {t.nickSave}
        </button>
      </div>
      {muted.length > 0 && (
        <>
          <h3 className="settings-section">{t.mutedTitle}</h3>
          <ul className="member-list">
            {muted.map((entry) => (
              <li key={entry.author}>
                <span className="who">
                  <span className="name">{entry.nick || "?"}</span>
                  <span className="fingerprint">{entry.author}</span>
                </span>
                <button
                  onClick={() =>
                    run(
                      () => api.unmuteAuthor(channel.id, entry.author),
                      () => {
                        setMuted((current) => current.filter((other) => other.author !== entry.author));
                        onUnmuted(entry.nick || entry.author);
                      },
                    )
                  }
                >
                  {t.unmute}
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
      <h3 className="settings-section">{t.shareCard}</h3>
      <p className="hint">{t.shareCardHint}</p>
      <button onClick={() => run(() => api.shareCard(channel.id), onShared)}>{t.shareCard}</button>
      <h3 className="settings-section danger">{t.leaveChannelTitle}</h3>
      <p className={error ? "hint error" : "hint"}>{error ?? t.leaveChannelHint}</p>
      <div className="dialog-actions">
        {confirmLeave ? (
          <button className="danger" onClick={() => run(() => api.leaveChannel(channel.id), onLeft)}>
            {t.leaveChannelNow}
          </button>
        ) : (
          <button className="danger-outline" onClick={() => setConfirmLeave(true)}>
            {t.leaveChannel}
          </button>
        )}
        <button className="primary" onClick={onClose}>
          {t.done}
        </button>
      </div>
    </Dialog>
  );
}

type NewChoice = "add" | "newGroup" | "joinChannel";

/** What the floating button on the chat list starts. */
export function NewDialog({ onClose, onChoose }: { onClose: () => void; onChoose: (choice: NewChoice) => void }) {
  const { t } = useI18n();
  const choices: { choice: NewChoice; icon: ReactNode; title: string; hint: string }[] = [
    { choice: "add", icon: <UserPlusIcon size={22} />, title: t.addContact, hint: t.addContactHint },
    { choice: "newGroup", icon: <UsersThreeIcon size={22} />, title: t.newGroup, hint: t.newGroupHint },
    { choice: "joinChannel", icon: <HashIcon size={22} />, title: t.joinChannel, hint: t.joinChannelHint },
  ];
  return (
    <Dialog title={t.newTitle} onClose={onClose}>
      <div className="group">
        {choices.map((item) => (
          <button key={item.choice} className="row-item" onClick={() => onChoose(item.choice)}>
            <span className="tile">{item.icon}</span>
            <span className="row-text">
              <b>{item.title}</b>
              <span>{item.hint}</span>
            </span>
          </button>
        ))}
      </div>
    </Dialog>
  );
}

/** A contact's fingerprint and how private the conversation is. */
export function ContactInfoDialog({ contact, onClose }: { contact: Contact; onClose: () => void }) {
  const { t } = useI18n();
  return (
    <Dialog title={contact.name} onClose={onClose}>
      <p className="hint">{t.compareFingerprint}</p>
      <p className="fingerprint large">{contact.fingerprint}</p>
      {!contact.request && (
        <>
          <h3 className="settings-section">{contact.secure ? t.secure : t.securePending}</h3>
          <p className="hint">{contact.secure ? t.secureHint : t.securePendingHint}</p>
        </>
      )}
      <div className="dialog-actions">
        <button className="primary" onClick={onClose}>
          {t.done}
        </button>
      </div>
    </Dialog>
  );
}

/** Where one of our messages went: each link it left by, and when it arrived and was read. */
export function JourneyDialog({ message, name, onClose }: { message: Message; name: string; onClose: () => void }) {
  const { t, language } = useI18n();
  const [journey, setJourney] = useState<Journey | null>(null);

  useEffect(() => {
    api.journey(message.id).then(setJourney);
  }, [message.id, message.via.length, message.delivered, message.read]);

  if (!journey) return null;
  const arrived = journey.deliveredAt !== null;
  const took = (arrived ? journey.deliveredAt! : Date.now() / 1000) - journey.createdAt;
  const devices = journey.steps
    .filter((step) => step.via === "lan" || step.via === "bluetooth")
    .reduce((sum, step) => sum + step.count, 0);
  const internet = journey.steps.some((step) => step.via === "internet");
  const time = (unix: number) => <time title={fullTime(unix, language)}>{clockTime(unix, language)}</time>;

  return (
    <Dialog title={t.journeyTitle} onClose={onClose}>
      <div className="journey-quote">
        <span className="mini-bubble">{message.body || t.file}</span>
        <span>{fullTime(message.createdAt, language)}</span>
      </div>
      <div className="journey-stats">
        <div className="stat hero">
          <b>{t.duration(took)}</b>
          <span>{arrived ? t.statArrivedIn : t.statOnTheWay}</span>
        </div>
        <div className="stat">
          <b>{devices}</b>
          <span>{t.statDevices}</span>
        </div>
        <div className="stat">
          <b>{internet ? t.statWithInternet : t.statWithoutInternet}</b>
          <span>{arrived ? t.statArrived : t.statOnTheWay}</span>
        </div>
      </div>
      <ol className="journey-steps">
        <li>
          <span className="node">
            <LockSimpleIcon size={15} weight="bold" />
          </span>
          <span className="row-text">
            <b>{t.journeyWritten}</b>
            <span>{t.journeyWrittenHint}</span>
          </span>
          {time(journey.createdAt)}
        </li>
        {journey.steps.map((step) => (
          <li key={step.via}>
            <span className="node">
              <ViaIcon via={step.via} size={15} />
            </span>
            <span className="row-text">
              <b>{t.journeyStep(step.via)}</b>
              <span>{t.journeyStepHint(step.via, step.count)}</span>
            </span>
            {time(step.at)}
          </li>
        ))}
        {arrived ? (
          <li>
            <span className="node done">
              <CheckIcon size={15} weight="bold" />
            </span>
            <span className="row-text">
              <b>{t.journeyArrived(name)}</b>
            </span>
            {time(journey.deliveredAt!)}
          </li>
        ) : (
          <li className="pending">
            <span className="node waiting">
              <HourglassMediumIcon size={15} weight="bold" />
            </span>
            <span className="row-text">
              <b>{t.journeyNotYet}</b>
              <span>{t.journeyNotYetHint}</span>
            </span>
          </li>
        )}
        {journey.readAt !== null && (
          <li>
            <span className="node done">
              <ChecksIcon size={15} weight="bold" />
            </span>
            <span className="row-text">
              <b>{t.read}</b>
            </span>
            {time(journey.readAt)}
          </li>
        )}
      </ol>
      <p className="journey-note">
        <ShieldCheckIcon size={16} weight="fill" />
        {t.journeyCarriers}
      </p>
    </Dialog>
  );
}

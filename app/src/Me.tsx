import { CaretRightIcon, QrCodeIcon } from "@phosphor-icons/react";
import { useEffect, useState, type FormEvent } from "react";
import { api, type Profile } from "./api";
import { Avatar } from "./Avatar";
import { MAX_NAME_BYTES, utf8Length } from "./format";
import { useI18n, type LanguageChoice } from "./i18n";
import { notificationsEnabled, setNotificationsEnabled } from "./prefs";

type Props = {
  profile: Profile;
  onRenamed: (profile: Profile) => void;
  onCard: () => void;
};

/** Who I am on this device, and the settings that belong to me rather than to a link. */
export function Me({ profile, onRenamed, onCard }: Props) {
  const { t, choice, setChoice, language } = useI18n();
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
      setError(String(failure).replace(/^Error: /, ""));
    }
  }

  async function toggleReadReceipts(on: boolean) {
    setReadReceipts(on);
    await api.setReadReceipts(on).catch((failure) => setError(String(failure).replace(/^Error: /, "")));
  }

  function toggleNotifications(on: boolean) {
    setNotifications(on);
    setNotificationsEnabled(on);
    api.setNotifications(on, language);
  }

  async function erase() {
    try {
      await api.eraseEverything();
    } catch (failure) {
      setError(String(failure).replace(/^Error: /, ""));
    }
  }

  return (
    <div className="tab-page">
      <header className="page-head">
        <h1 className="page-title">{t.tabMe}</h1>
      </header>

      <div className="identity">
        <Avatar name={profile.name} size={64} />
        <div>
          <strong>{profile.name}</strong>
          <span className="fingerprint">{profile.fingerprint}</span>
        </div>
      </div>

      <div className="group">
        <button className="row-item" onClick={onCard}>
          <span className="tile">
            <QrCodeIcon size={20} />
          </span>
          <span className="row-text">
            <b>{t.myCard}</b>
            <span>{t.myCardHint}</span>
          </span>
          <CaretRightIcon className="go" size={16} weight="bold" />
        </button>
      </div>

      <form className="group form-group" onSubmit={rename}>
        <label htmlFor="rename">{t.yourName}</label>
        <div className="inline-field">
          <input id="rename" value={name} onChange={(event) => setName(event.target.value)} aria-invalid={tooLong} />
          <button className="primary" disabled={tooLong || trimmed === "" || trimmed === profile.name}>
            {t.save}
          </button>
        </div>
        <p className={tooLong ? "hint error" : "hint"}>{tooLong ? t.nameTooLong : t.renameHint}</p>
      </form>

      <h2 className="group-label">{t.privacy}</h2>
      <div className="group">
        <label className="row-item toggle">
          <span className="row-text">
            <b>{t.readReceipts}</b>
          </span>
          <input
            type="checkbox"
            checked={readReceipts ?? true}
            disabled={readReceipts === null}
            onChange={(event) => toggleReadReceipts(event.target.checked)}
          />
        </label>
        <label className="row-item toggle">
          <span className="row-text">
            <b>{t.notifications}</b>
          </span>
          <input type="checkbox" checked={notifications} onChange={(event) => toggleNotifications(event.target.checked)} />
        </label>
      </div>

      <div className="group spaced">
        <label className="row-item">
          <span className="row-text">
            <b>{t.language}</b>
          </span>
          <select value={choice} onChange={(event) => setChoice(event.target.value as LanguageChoice)} aria-label={t.language}>
            <option value="system">{t.languageSystem}</option>
            <option value="en">English</option>
            <option value="tr">Türkçe</option>
          </select>
        </label>
      </div>

      <h2 className="group-label danger">{t.eraseTitle}</h2>
      <div className="group form-group">
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
            <button className="danger-outline" onClick={() => setConfirmErase(true)}>
              {t.eraseButton}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

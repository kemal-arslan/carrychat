import { CaretRightIcon, EyeIcon, QrCodeIcon, UserPlusIcon } from "@phosphor-icons/react";
import type { Contact, NearbyPerson } from "./api";
import { Avatar } from "./Avatar";
import { useI18n } from "./i18n";
import { ViaIcon } from "./Via";

type Props = {
  contacts: Contact[];
  activeId: number | null;
  /** Whether we are visible nearby; null until known. */
  nearbyOn: boolean | null;
  nearby: NearbyPerson[];
  onNearbyOn: (on: boolean) => void;
  onNearby: (person: NearbyPerson) => void;
  onOpen: (id: number) => void;
  onAdd: () => void;
  onCard: () => void;
};

/** Everyone whose card this device holds, with their fingerprints. */
export function People({ contacts, activeId, nearbyOn, nearby, onNearbyOn, onNearby, onOpen, onAdd, onCard }: Props) {
  const { t, language } = useI18n();
  const requests = contacts.filter((contact) => contact.request);
  const accepted = contacts
    .filter((contact) => !contact.request)
    .sort((a, b) => a.name.localeCompare(b.name, language));

  function row(contact: Contact) {
    return (
      <button
        key={contact.id}
        className={contact.id === activeId ? "chat-row active" : "chat-row"}
        onClick={() => onOpen(contact.id)}
      >
        <Avatar name={contact.name} size={44} />
        <span className="row-main">
          <span className="row-top">
            <span className="name">{contact.name}</span>
            {contact.request && <span className="tag accent">{t.requestTag}</span>}
          </span>
          <span className="row-bottom">
            <span className="preview fingerprint">{contact.fingerprint}</span>
          </span>
        </span>
      </button>
    );
  }

  return (
    <div className="tab-page">
      <header className="page-head">
        <h1 className="page-title">{t.contacts}</h1>
        <div className="page-actions">
          <button className="icon-button" onClick={onAdd} aria-label={t.addContact} title={t.addContact}>
            <UserPlusIcon size={22} />
          </button>
          <button className="icon-button" onClick={onCard} aria-label={t.myCard} title={t.myCard}>
            <QrCodeIcon size={22} />
          </button>
        </div>
      </header>
      <h2 className="group-label">{t.nearbyTitle}</h2>
      <div className="group">
        <label className="row-item toggle">
          <span className="tile">
            <EyeIcon size={20} />
          </span>
          <span className="row-text">
            <b>{t.nearbyVisible}</b>
            <span>{t.nearbyVisibleHint}</span>
          </span>
          <input
            type="checkbox"
            checked={nearbyOn ?? false}
            disabled={nearbyOn === null}
            onChange={(event) => onNearbyOn(event.target.checked)}
          />
        </label>
        {nearbyOn &&
          nearby.map((person) => (
            <button key={person.session} className="row-item" onClick={() => onNearby(person)}>
              <Avatar name={person.name} size={36} />
              <span className="row-text">
                <b>{person.name}</b>
                <span className="via-line">
                  <ViaIcon via={person.link} size={12} />
                  {t.nearbyVia(person.link)}
                </span>
              </span>
              <CaretRightIcon className="go" size={16} weight="bold" />
            </button>
          ))}
        {nearbyOn && nearby.length === 0 && (
          <div className="row-item">
            <span className="row-text">
              <span>{t.nearbyNone}</span>
            </span>
          </div>
        )}
      </div>

      {requests.length > 0 && (
        <>
          <h2 className="group-label">{t.requests}</h2>
          <div className="rows">{requests.map(row)}</div>
        </>
      )}
      {accepted.length > 0 && <h2 className="group-label">{t.contacts}</h2>}
      <div className="rows">{accepted.map(row)}</div>
      {contacts.length === 0 && <p className="empty">{t.sidebarEmpty}</p>}
    </div>
  );
}

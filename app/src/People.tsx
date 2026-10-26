import { QrCodeIcon, UserPlusIcon } from "@phosphor-icons/react";
import type { Contact } from "./api";
import { Avatar } from "./Avatar";
import { useI18n } from "./i18n";

type Props = {
  contacts: Contact[];
  activeId: number | null;
  onOpen: (id: number) => void;
  onAdd: () => void;
  onCard: () => void;
};

/** Everyone whose card this device holds, with their fingerprints. */
export function People({ contacts, activeId, onOpen, onAdd, onCard }: Props) {
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
      {requests.length > 0 && (
        <>
          <h2 className="group-label">{t.requests}</h2>
          <div className="rows">{requests.map(row)}</div>
        </>
      )}
      {requests.length > 0 && accepted.length > 0 && <h2 className="group-label">{t.contacts}</h2>}
      <div className="rows">{accepted.map(row)}</div>
      {contacts.length === 0 && <p className="empty">{t.sidebarEmpty}</p>}
    </div>
  );
}

import { useI18n } from "./i18n";

/** Back to the list, on a phone where the conversation covers it. */
export function BackButton({ onBack }: { onBack: () => void }) {
  const { t } = useI18n();
  return (
    <button className="icon-button back-button" onClick={onBack} aria-label={t.back} title={t.back}>
      <svg width="20" height="20" viewBox="0 0 24 24" aria-hidden>
        <path fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" d="M15 5l-7 7 7 7" />
      </svg>
    </button>
  );
}

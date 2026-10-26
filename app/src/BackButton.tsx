import { ArrowLeftIcon } from "@phosphor-icons/react";
import { useI18n } from "./i18n";

/** Back to the list, on a phone where the conversation covers it. */
export function BackButton({ onBack }: { onBack: () => void }) {
  const { t } = useI18n();
  return (
    <button className="icon-button back-button" onClick={onBack} aria-label={t.back} title={t.back}>
      <ArrowLeftIcon size={22} />
    </button>
  );
}

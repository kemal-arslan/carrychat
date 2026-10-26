import { useEffect, useState } from "react";
import { api, type FileView } from "./api";
import { useI18n } from "./i18n";

/** A photo or voice message in a conversation, or how much of it is here. */
export function FileBubble({ file }: { file: FileView }) {
  const { t } = useI18n();
  const [url, setUrl] = useState<string | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (file.ready) api.fileUrl(file.id).then(setUrl);
  }, [file.id, file.ready]);

  useEffect(() => {
    if (!open) return;
    const close = (event: KeyboardEvent) => event.key === "Escape" && setOpen(false);
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [open]);

  if (!file.ready) {
    const lost = Date.now() / 1000 >= file.expiresAt;
    return (
      <p className="file-pending">
        {lost ? t.fileLost : t.fileArriving(kind(file.mime, t), file.received, file.chunks)}
      </p>
    );
  }
  if (!url) return null;

  const save = <button className="file-save" onClick={() => api.saveFile(file)}>{t.saveFile}</button>;
  if (file.mime.startsWith("image/")) {
    return (
      <>
        <button className="file-open" onClick={() => setOpen(true)} aria-label={t.photo}>
          <img className="file-image" src={url} alt={t.photo} />
        </button>
        {open && (
          <div className="lightbox" role="dialog" aria-label={t.photo} onClick={() => setOpen(false)}>
            <img src={url} alt={t.photo} onClick={(event) => event.stopPropagation()} />
            <div className="lightbox-actions" onClick={(event) => event.stopPropagation()}>
              {save}
              <button className="primary" onClick={() => setOpen(false)}>
                {t.close}
              </button>
            </div>
          </div>
        )}
      </>
    );
  }
  if (file.mime.startsWith("audio/")) {
    return (
      <div className="file-audio">
        <audio src={url} controls preload="metadata" />
        {save}
      </div>
    );
  }
  return (
    <p className="file-other">
      {t.fileOther(file.mime, Math.ceil(file.size / 1024))} {save}
    </p>
  );
}

function kind(mime: string, t: ReturnType<typeof useI18n>["t"]): string {
  if (mime.startsWith("image/")) return t.photo;
  if (mime.startsWith("audio/")) return t.voiceMessage;
  return t.file;
}

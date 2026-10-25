import { useEffect, useState } from "react";
import { api, type FileView } from "./api";
import { useI18n } from "./i18n";

/** A photo or voice message in a conversation, or how much of it is here. */
export function FileBubble({ file }: { file: FileView }) {
  const { t } = useI18n();
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    if (file.ready) api.fileUrl(file.id).then(setUrl);
  }, [file.id, file.ready]);

  if (!file.ready) {
    const lost = Date.now() / 1000 >= file.expiresAt;
    return (
      <p className="file-pending">
        {lost ? t.fileLost : t.fileArriving(kind(file.mime, t), file.received, file.chunks)}
      </p>
    );
  }
  if (!url) return null;
  if (file.mime.startsWith("image/")) {
    return (
      <a href={url} target="_blank" rel="noreferrer">
        <img className="file-image" src={url} alt={t.photo} />
      </a>
    );
  }
  if (file.mime.startsWith("audio/")) {
    return <audio className="file-audio" src={url} controls preload="metadata" />;
  }
  return (
    <a className="file-other" href={url} download>
      {t.fileOther(file.mime, Math.ceil(file.size / 1024))}
    </a>
  );
}

function kind(mime: string, t: ReturnType<typeof useI18n>["t"]): string {
  if (mime.startsWith("image/")) return t.photo;
  if (mime.startsWith("audio/")) return t.voiceMessage;
  return t.file;
}

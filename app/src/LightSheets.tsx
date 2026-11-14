import { FlashlightIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState, type PointerEvent } from "react";
import type { LightWatch } from "./api";
import { useI18n } from "./i18n";
import { BOX, keepScreenOn, sendByLight, sendingLight, stopSendingLight, watchLight, type Aim, type Zoom } from "./light";
import { Sheet } from "./Sheet";

const errorText = (failure: unknown) => String(failure).replace(/^Error: /, "");

/** Blinks one of our messages with the flashlight, pass after pass, until closed. */
export function LightSendSheet({ messageId, onClose }: { messageId: number; onClose: () => void }) {
  const { t } = useI18n();
  // How long a pass takes, once the flashlight blinks, and when it began.
  const [pass, setPass] = useState<number | null>(null);
  const [since, setSince] = useState(0);
  const [now, setNow] = useState(Date.now());
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    keepScreenOn(true);
    sendByLight(messageId)
      .then((length) => {
        if (cancelled) return;
        setSince(Date.now());
        setPass(length);
      })
      .catch((failure) => !cancelled && setError(`${t.lightFailed} ${errorText(failure)}`));
    return () => {
      cancelled = true;
      stopSendingLight();
      keepScreenOn(false);
    };
  }, [messageId]);

  useEffect(() => {
    if (pass === null) return;
    const timer = window.setInterval(() => {
      setNow(Date.now());
      if (!sendingLight()) setError(t.lightStopped);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [pass]);

  const elapsed = Math.max(0, now - since);
  const passes = pass ? Math.floor(elapsed / pass) + 1 : 1;
  const share = pass ? (elapsed % pass) / pass : 0;
  const left = pass ? Math.ceil((pass - (elapsed % pass)) / 1000) : 0;

  return (
    <Sheet title={t.lightSendingTitle} onClose={onClose}>
      <div className={error ? "light-beacon" : "light-beacon on"} aria-hidden>
        <FlashlightIcon size={36} weight="fill" />
      </div>
      {pass !== null && <p className="hint">{t.lightSendingHelp(t.duration(Math.ceil(pass / 60_000) * 60))}</p>}
      {pass !== null && !error && (
        <>
          <div className="light-progress" role="progressbar" aria-valuenow={Math.round(share * 100)}>
            <span style={{ width: `${share * 100}%` }} />
          </div>
          <p className="hint">
            {t.lightPass(passes)} · {t.lightPassLeft(t.duration(left))}
          </p>
        </>
      )}
      {error && <p className="hint error">{error}</p>}
      <div className="dialog-actions">
        <button className="primary" onClick={onClose}>
          {t.soundStop}
        </button>
      </div>
    </Sheet>
  );
}

/** Shows the back camera and reads the flashlight of a phone in sight, while open. */
export function LightWatchSheet({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  // The sheet puts its content on the page after it opens, so watching waits for the video to be there.
  const [video, setVideo] = useState<HTMLVideoElement | null>(null);
  const aim = useRef<Aim>({ x: 0.5, y: 0.5 });
  const [box, setBox] = useState<Aim>(aim.current);
  const [shape, setShape] = useState<{ width: number; height: number } | null>(null);
  const [watch, setWatch] = useState<LightWatch | null>(null);
  const [zoom, setZoom] = useState<Zoom | null>(null);
  const [zoomValue, setZoomValue] = useState(1);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!video) return;
    let stop: (() => void) | null = null;
    let cancelled = false;
    keepScreenOn(true);
    watchLight(video, () => aim.current, setWatch)
      .then((started) => {
        if (cancelled) return started.stop();
        stop = started.stop;
        setZoom(started.zoom);
        if (started.zoom) setZoomValue(started.zoom.value);
      })
      .catch(() => !cancelled && setError(t.lightNoCamera));
    return () => {
      cancelled = true;
      stop?.();
      keepScreenOn(false);
    };
  }, [video]);

  function place(event: PointerEvent<HTMLDivElement>) {
    const area = event.currentTarget.getBoundingClientRect();
    const next = {
      x: Math.min(1, Math.max(0, (event.clientX - area.left) / area.width)),
      y: Math.min(1, Math.max(0, (event.clientY - area.top) / area.height)),
    };
    aim.current = next;
    setBox(next);
  }

  // The box is a square of the picture's shorter side, kept inside the picture.
  const side = shape ? BOX * Math.min(shape.width, shape.height) : 0;
  const boxWidth = shape ? side / shape.width : BOX;
  const boxHeight = shape ? side / shape.height : BOX;
  const boxLeft = Math.min(Math.max(0, box.x - boxWidth / 2), 1 - boxWidth);
  const boxTop = Math.min(Math.max(0, box.y - boxHeight / 2), 1 - boxHeight);
  const seen = watch !== null && watch.contrast >= 6;
  const status =
    watch === null ? null : !seen ? t.lightNone : watch.of === 0 ? t.lightSeen : t.lightReading(watch.blocks, watch.of);

  return (
    <Sheet title={t.lightName} onClose={onClose}>
      <p>{t.lightAim}</p>
      <div
        className="light-view"
        data-vaul-no-drag
        onPointerDown={place}
        style={shape ? { aspectRatio: `${shape.width} / ${shape.height}` } : undefined}
      >
        <video
          ref={setVideo}
          muted
          playsInline
          onLoadedMetadata={(event) =>
            setShape({ width: event.currentTarget.videoWidth, height: event.currentTarget.videoHeight })
          }
        />
        <span
          className={seen ? "light-box seen" : "light-box"}
          style={{
            left: `${boxLeft * 100}%`,
            top: `${boxTop * 100}%`,
            width: `${boxWidth * 100}%`,
            height: `${boxHeight * 100}%`,
          }}
        />
      </div>
      {zoom && (
        <label className="light-zoom">
          <span>{t.lightZoom}</span>
          <input
            type="range"
            min={zoom.min}
            max={zoom.max}
            step={zoom.step}
            value={zoomValue}
            onChange={(event) => {
              const value = Number(event.target.value);
              setZoomValue(value);
              zoom.set(value);
            }}
          />
        </label>
      )}
      {error ? (
        <p className="hint error">{error}</p>
      ) : (
        status && (
          <p className={seen ? "hint live" : "hint"} aria-live="polite">
            {status}
            {watch !== null && watch.frames > 0 && ` · ${t.lightParts(watch.frames)}`}
          </p>
        )
      )}
      <p className="hint">{t.lightWhere}</p>
      <div className="dialog-actions">
        <button className="primary" onClick={onClose}>
          {t.close}
        </button>
      </div>
    </Sheet>
  );
}

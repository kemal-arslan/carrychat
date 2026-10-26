import { HashIcon, UsersThreeIcon } from "@phosphor-icons/react";
import type { CSSProperties } from "react";
import { useI18n } from "./i18n";

const HUES = ["#E8590C", "#2F5BFF", "#0E9F6E", "#7048E8", "#D6336C", "#0C8599"];

/** A colour that stays the same for a name, so a person is recognisable at a glance. */
function hue(name: string): string {
  let hash = 0;
  for (const char of name) hash = (hash * 31 + char.codePointAt(0)!) | 0;
  return HUES[Math.abs(hash) % HUES.length];
}

type Props = { name: string; kind?: "contact" | "group" | "channel"; size?: number };

export function Avatar({ name, kind = "contact", size = 50 }: Props) {
  const { language } = useI18n();
  const style = { "--hue": hue(name), width: size, height: size, fontSize: size * 0.38 } as CSSProperties;
  return (
    <span className="avatar" style={style} aria-hidden>
      {kind === "group" ? (
        <UsersThreeIcon weight="fill" size={size * 0.46} />
      ) : kind === "channel" ? (
        <HashIcon weight="bold" size={size * 0.42} />
      ) : (
        name.slice(0, 1).toLocaleUpperCase(language)
      )}
    </span>
  );
}

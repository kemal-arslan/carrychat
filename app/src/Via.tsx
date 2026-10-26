import { BluetoothIcon, BroadcastIcon, GlobeSimpleIcon, UsbIcon, WifiHighIcon, type Icon } from "@phosphor-icons/react";
import type { Via } from "./api";

const ICONS: Record<Via, Icon> = {
  lan: WifiHighIcon,
  bluetooth: BluetoothIcon,
  internet: GlobeSimpleIcon,
  lora: BroadcastIcon,
  file: UsbIcon,
};

export function ViaIcon({ via, size = 13 }: { via: Via; size?: number }) {
  const ViaGlyph = ICONS[via];
  return <ViaGlyph size={size} weight="bold" />;
}

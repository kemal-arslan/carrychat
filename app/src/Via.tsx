import {
  BluetoothIcon,
  CellTowerIcon,
  DevicesIcon,
  GlobeSimpleIcon,
  GraphIcon,
  ShareNetworkIcon,
  UsbIcon,
  WifiHighIcon,
  type Icon,
} from "@phosphor-icons/react";
import type { Via } from "./api";

const ICONS: Record<Via, Icon> = {
  lan: WifiHighIcon,
  bluetooth: BluetoothIcon,
  wifiAware: DevicesIcon,
  wifiDirect: ShareNetworkIcon,
  internet: GlobeSimpleIcon,
  lora: CellTowerIcon,
  reticulum: GraphIcon,
  file: UsbIcon,
};

export function ViaIcon({ via, size = 13 }: { via: Via; size?: number }) {
  const ViaGlyph = ICONS[via];
  return <ViaGlyph size={size} weight="bold" />;
}

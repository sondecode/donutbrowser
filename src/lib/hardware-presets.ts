import type { WayfernFingerprintConfig, WayfernOS } from "@/types";
import hardwarePresetData from "./hardware-presets-data.json";

export type HardwarePresetGroupId = "macbook_m" | "mac_mini" | "us_laptop";

export type HardwarePresetId =
  | "macbook_air_m1_13"
  | "macbook_pro_m1_13"
  | "macbook_air_m2_13"
  | "macbook_pro_m2_14"
  | "macbook_pro_m2_16"
  | "macbook_air_m3_13"
  | "macbook_air_m3_15"
  | "macbook_pro_m3_14"
  | "macbook_pro_m3_16"
  | "macbook_pro_m4_16"
  | "mac_mini_intel_2018_6"
  | "mac_mini_intel_2018_8"
  | "mac_mini_m1_2020_8"
  | "mac_mini_m1_2020_16"
  | "mac_mini_m2_2023_8"
  | "mac_mini_m2_2023_16"
  | "mac_mini_m2_pro_2023"
  | "mac_mini_m4_2024_8"
  | "mac_mini_m4_2024_16"
  | "mac_mini_m4_pro_2024"
  | "dell_xps_13"
  | "dell_xps_15"
  | "hp_spectre_x360_14"
  | "lenovo_thinkpad_x1_carbon"
  | "lenovo_thinkpad_t14"
  | "microsoft_surface_laptop_5"
  | "microsoft_surface_laptop_studio_2"
  | "asus_zenbook_14"
  | "acer_swift_14"
  | "lenovo_yoga_7i";

export interface HardwarePreset {
  id: HardwarePresetId;
  group: HardwarePresetGroupId;
  os: WayfernOS;
  labelKey: string;
  config: Partial<WayfernFingerprintConfig>;
}

const data = hardwarePresetData as {
  presetKeys: (keyof WayfernFingerprintConfig)[];
  groups: {
    id: HardwarePresetGroupId;
    labelKey: string;
  }[];
  presets: HardwarePreset[];
};

export const HARDWARE_PRESET_KEYS = data.presetKeys;

export const HARDWARE_PRESET_GROUPS = data.groups;

export const HARDWARE_PRESETS = data.presets;

export function applyHardwarePreset(
  currentConfig: WayfernFingerprintConfig,
  presetId: HardwarePresetId,
): WayfernFingerprintConfig {
  const preset = HARDWARE_PRESETS.find((item) => item.id === presetId);
  if (!preset) return currentConfig;

  const nextConfig = { ...currentConfig };
  for (const key of HARDWARE_PRESET_KEYS) {
    delete nextConfig[key];
  }

  return { ...nextConfig, ...preset.config };
}

export function matchHardwarePreset(
  config: WayfernFingerprintConfig,
  os: WayfernOS | undefined,
): HardwarePresetId | null {
  for (const preset of HARDWARE_PRESETS) {
    if (preset.os !== os) continue;
    const entries = Object.entries(preset.config) as [
      keyof WayfernFingerprintConfig,
      unknown,
    ][];
    const matches = entries.every(([key, value]) => config[key] === value);
    if (matches) return preset.id;
  }
  return null;
}

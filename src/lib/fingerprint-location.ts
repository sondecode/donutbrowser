import type { WayfernConfig, WayfernFingerprintConfig } from "@/types";

export const MANUAL_LOCATION_FIELDS = [
  "timezone",
  "timezoneOffset",
  "latitude",
  "longitude",
  "accuracy",
] as const satisfies readonly (keyof WayfernFingerprintConfig)[];

export function parseFingerprintConfig(
  config: WayfernConfig,
): WayfernFingerprintConfig {
  if (!config.fingerprint) return {};
  try {
    return JSON.parse(config.fingerprint) as WayfernFingerprintConfig;
  } catch {
    return {};
  }
}

export function isManualLocationComplete(config: WayfernConfig): boolean {
  if (config.geoip !== false) return true;

  const fingerprint = parseFingerprintConfig(config);
  return (
    typeof fingerprint.timezone === "string" &&
    fingerprint.timezone.trim().length > 0 &&
    Number.isFinite(fingerprint.timezoneOffset) &&
    Number.isFinite(fingerprint.latitude) &&
    Number.isFinite(fingerprint.longitude) &&
    Number.isFinite(fingerprint.accuracy)
  );
}

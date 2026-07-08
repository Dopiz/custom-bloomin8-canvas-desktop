// Mirrors the Rust structs in src-tauri/src/config.rs and
// src-tauri/src/device/types.rs. Kept intentionally loose (extra fields
// tolerated) since DeviceInfo/DeviceState use `#[serde(flatten)] extra` on
// the Rust side.

export interface DeviceEntry {
  id: string;
  name: string;
  lan_ip: string;
  ble_name: string;
}

export interface AppConfig {
  devices: DeviceEntry[];
  active_device_id: string | null;
}

/** One BLE peripheral matched by `bleScan` (mirrors Rust `wake::BleMatch`). */
export interface BleMatch {
  name: string;
  rssi: number;
}

export interface DeviceInfo {
  width: number;
  height: number;
  battery?: number | null;
  image?: string | null;
  gallery?: string | null;
  max_idle?: number | null;
  name?: string;
  sleep_duration?: number;
  idx_wake_sens?: number;
  /** WiFi SSID the device is connected to (from `/deviceInfo.sta_ssid`). */
  sta_ssid?: string;
  /** WiFi signal strength in dBm (from `/deviceInfo.sta_rssi`). */
  sta_rssi?: number;
  /** The device's LAN IP as it reports it (from `/deviceInfo.sta_ip`). */
  sta_ip?: string;
  [key: string]: unknown;
}

export interface DeviceSettingsUpdate {
  name?: string;
  sleep_duration?: number;
  max_idle?: number;
  idx_wake_sens?: number;
}

export interface GallerySummary {
  name: string;
  [key: string]: unknown;
}

export interface GalleryImage {
  name: string;
  [key: string]: unknown;
}

export interface PlaylistSummary {
  name: string;
  [key: string]: unknown;
}

/** Raw playlist document shape used by this app's editor — the device's
 * actual schema is looser (`GET /playlist` is passed through as-is), but the
 * editor only ever writes/reads this shape. */
export interface PlaylistDoc {
  name: string;
  type?: string;
  list: string[];
  [key: string]: unknown;
}

// --- Widgets page ----------------------------------------------------
// Mirrors src-tauri/src/widgets/config.rs's `WidgetConfig`/`WidgetRenderConfig`
// (serde `tag = "kind"`, flattened into `WidgetRenderConfig`).

export interface CryptoWidgetConfig {
  kind: "crypto";
  symbols: string[];
  range: "24h" | "7d" | "30d";
}

export interface WeatherWidgetConfig {
  kind: "weather";
  lat: number;
  lon: number;
  city: string;
  force_icon?: string | null;
}

export interface CountdownWidgetConfig {
  kind: "countdown";
  /** `YYYY-MM-DD` */
  target_date: string;
  title: string;
  bg_query: string;
  bg_photo?: string | null;
}

/** A fixed image from the local library, pushed with display settings. Mirrors
 * the Rust `WidgetConfig::Image` variant. `orientation`/`rotate` come from the
 * `WidgetRenderConfig` wrapper (a duplicate `orientation` here would collide
 * with it on the flattened JSON key); rotation is always clockwise, matching
 * the push dialog. */
export interface ImageWidgetConfig {
  kind: "image";
  library_id: string;
  mode: DisplayMode;
  border: BorderColor;
}

export type WidgetConfig =
  | CryptoWidgetConfig
  | WeatherWidgetConfig
  | CountdownWidgetConfig
  | ImageWidgetConfig;

export type PanelOrientation = "portrait" | "landscape";
export type RotateDirection = "cw" | "ccw";
export type DisplayMode = "auto" | "fit" | "fill";
export type BorderColor = "white" | "black";

export type WidgetRenderConfig = WidgetConfig & {
  orientation: PanelOrientation;
  rotate: RotateDirection;
};

// --- Schedules page ---------------------------------------------------
// Mirrors src-tauri/src/scheduler.rs's `Schedule`/`HistoryEntry`.

export interface Schedule {
  id: string;
  name: string;
  /** The device this schedule pushes to (a `DeviceEntry.id`). Empty on legacy
   * schedules from before per-device scheduling; the backend resolves an empty
   * value to the active device. */
  device_id: string;
  widget: WidgetRenderConfig;
  /** Cron expression (v1: raw string, friendly picker planned). */
  cron: string;
  enabled: boolean;
}

export type RunStatus = "success" | "failed" | "skipped";

export interface HistoryEntry {
  schedule_id: string;
  /** Naive local datetime, e.g. `2026-07-07T09:00:00`. */
  started_at: string;
  finished_at: string;
  status: RunStatus;
  error?: string | null;
  filename?: string | null;
}

import { invoke } from "@tauri-apps/api/core";
import type {
  AppConfig,
  BleMatch,
  DeviceInfo,
  DeviceSettingsUpdate,
  GalleryImage,
  GallerySummary,
  HistoryEntry,
  PanelOrientation,
  PlaylistSummary,
  RotateDirection,
  Schedule,
  WidgetRenderConfig,
} from "../types";

export function getConfig(): Promise<AppConfig> {
  return invoke("get_config");
}

export function saveConfig(config: AppConfig): Promise<void> {
  return invoke("save_config", { config });
}

export function fetchDeviceInfo(): Promise<DeviceInfo> {
  return invoke("device_info");
}

export function deviceWake(): Promise<void> {
  return invoke("device_wake");
}

/** Scan BLE for Bloomin8-like peripherals whose advertised name contains
 * `hint` (empty -> "Bloomin8"), strongest signal first. Never rejects — an
 * empty array means nothing was found (asleep / out of range / no permission). */
export function bleScan(hint: string): Promise<BleMatch[]> {
  return invoke("ble_scan", { hint });
}

export function deviceSleep(): Promise<void> {
  return invoke("device_sleep");
}

export function deviceReboot(): Promise<void> {
  return invoke("device_reboot");
}

export function deviceClearScreen(): Promise<void> {
  return invoke("device_clear_screen");
}

export function deviceSetSettings(settings: DeviceSettingsUpdate): Promise<void> {
  return invoke("device_set_settings", { settings });
}

/** Remove a deleted device's local data (its on-disk image cache + last-push
 * record). Schedules are removed separately by the caller. */
export function purgeDeviceData(deviceId: string): Promise<void> {
  return invoke("purge_device_data", { deviceId });
}

/** The last image pushed to the *active* device — its filename and the
 * orientation it was pushed with — or `null` if nothing has been pushed yet.
 * Used by the Device hero to render a landscape image the right way up. */
export function lastPush(): Promise<{
  filename: string;
  orientation: PanelOrientation;
} | null> {
  return invoke("last_push");
}

// --- Gallery page ---------------------------------------------------

export function galleryList(): Promise<GallerySummary[]> {
  return invoke("gallery_list");
}

export function galleryCreate(name: string): Promise<void> {
  return invoke("gallery_create", { name });
}

export function galleryDelete(name: string): Promise<void> {
  return invoke("gallery_delete", { name });
}

export function galleryImages(
  gallery: string,
  offset: number,
  limit: number,
): Promise<GalleryImage[]> {
  return invoke("gallery_images", { gallery, offset, limit });
}

export function imageDelete(gallery: string, image: string): Promise<void> {
  return invoke("image_delete", { gallery, image });
}

export function showImage(gallery: string, image: string): Promise<void> {
  return invoke("show_image", { gallery, image });
}

export function showGallery(gallery: string): Promise<void> {
  return invoke("show_gallery", { gallery });
}

export function showPlaylist(playlist: string): Promise<void> {
  return invoke("show_playlist", { playlist });
}

export function showNext(): Promise<void> {
  return invoke("show_next");
}

export function playlistList(): Promise<PlaylistSummary[]> {
  return invoke("playlist_list");
}

export function playlistGet(name: string): Promise<unknown> {
  return invoke("playlist_get", { name });
}

export function playlistSave(name: string, body: unknown): Promise<void> {
  return invoke("playlist_save", { name, body });
}

export function playlistDelete(name: string): Promise<void> {
  return invoke("playlist_delete", { name });
}

// --- Widgets page (preview + push) -----------------------------------

/** Renders `req` and returns a `data:image/jpeg;base64,...` URL — never
 * touches the device. */
export function previewWidget(req: WidgetRenderConfig): Promise<string> {
  return invoke("preview_widget", { req });
}

/** Renders `req` and pushes it to the active device (wake-if-needed ->
 * render -> upload_and_show); resolves with the filename now displayed. */
export function pushWidget(req: WidgetRenderConfig): Promise<string> {
  return invoke("push_widget", { req });
}

// --- Local image library + push-with-display-settings ---------------------

export interface LibraryItem {
  id: string;
  name: string;
  /** millis since epoch */
  added: number;
  ext: string;
}

/** Display settings applied client-side at push time (the device API has no
 * such params). `mode`: auto = fill portrait / fit landscape. */
export type DisplayMode = "auto" | "fit" | "fill";
export type BorderColor = "white" | "black";

export interface DisplaySettings {
  orientation: PanelOrientation;
  rotate: RotateDirection;
  mode: DisplayMode;
  border: BorderColor;
}

export function libraryList(): Promise<LibraryItem[]> {
  return invoke("library_list");
}

/** Save an uploaded original (from a file `<input>` data URL) into the local
 * library. Does NOT push to the device. */
export function libraryAdd(name: string, dataUrl: string): Promise<LibraryItem> {
  return invoke("library_add", { name, dataUrl });
}

export function libraryDelete(id: string): Promise<void> {
  return invoke("library_delete", { id });
}

/** The original bytes of a library image as a data URL (for thumbnail + as the
 * source for preview/push). */
export function libraryImage(id: string): Promise<string> {
  return invoke("library_image", { id });
}

/** Process `source` (a data URL) with the display settings and return a
 * panel-sized JPEG data URL for the push dialog's live preview. */
export function previewImage(source: string, s: DisplaySettings): Promise<string> {
  return invoke("preview_image", {
    source,
    orientation: s.orientation,
    rotate: s.rotate,
    mode: s.mode,
    border: s.border,
  });
}

/** Process `source` with the display settings and push it to the device.
 * Resolves with the filename now displayed. */
export function pushImage(source: string, s: DisplaySettings): Promise<string> {
  return invoke("push_image", {
    source,
    orientation: s.orientation,
    rotate: s.rotate,
    mode: s.mode,
    border: s.border,
  });
}

// --- Device image fetch (Gallery thumbnails + Device hero) ----------------

/** One full-resolution copy per image is fetched and cached; callers size it
 * down with CSS. In-flight promises are shared so concurrent grid cells make a
 * single call; the Rust side also caches bytes on disk forever (filenames are
 * never reused), so an image is pulled from the device at most
 * once, ever. Keyed by `gallery/name`. */
const imageCache = new Map<string, Promise<string>>();
/** Resolved data URLs, kept synchronously so a re-mounted <DeviceImage> can
 * paint instantly (no spinner flash) instead of awaiting a microtask. */
const resolvedImages = new Map<string, string>();

/** Active device id, folded into every image-cache key so each device keeps
 * its own bucket — switching devices doesn't clear or collide caches, and
 * switching back still hits. App sets this on load and on every switch. */
let activeDeviceKey = "";

export function setActiveDeviceKey(id: string): void {
  activeDeviceKey = id;
}

function imageKey(gallery: string, name: string): string {
  return `${activeDeviceKey}/${gallery}/${name}`;
}

/** Synchronously returns an already-loaded image's data URL, if cached this
 * session — lets callers seed initial state without a loading flash. */
export function cachedImage(gallery: string, name: string): string | undefined {
  return resolvedImages.get(imageKey(gallery, name));
}

/** Fetch a device image as a `data:image/jpeg;base64,...` URL (full size;
 * display size is a CSS concern). Cached in-memory this session on top of the
 * Rust disk cache. */
export function fetchImage(gallery: string, name: string): Promise<string> {
  const key = imageKey(gallery, name);
  const hit = imageCache.get(key);
  if (hit) return hit;
  const p = invoke<string>("fetch_image", { gallery, name })
    .then((url) => {
      resolvedImages.set(key, url);
      return url;
    })
    .catch((e) => {
      imageCache.delete(key); // don't cache failures — allow retry
      throw e;
    });
  imageCache.set(key, p);
  return p;
}

/** Approximate location from the machine's public IP (Weather location
 * fallback). Rejects with a displayable message on failure. */
export function ipGeolocation(): Promise<{ lat: number; lon: number; city: string }> {
  return invoke("ip_geolocation");
}

/** Reverse-geocode lat/lon to a city label (cosmetic only — weather works from
 * lat/lon alone). Resolves with "" when no name is available. */
export function reverseGeocode(lat: number, lon: number): Promise<string> {
  return invoke("reverse_geocode", { lat, lon });
}

/** Precise coordinates from the OS via the WKWebView's `navigator.geolocation`
 * (Core Location on macOS — requires the location permission grant). Rejects on
 * denial / unavailability / timeout; callers leave the location empty in that
 * case. */
export function browserGeolocation(
  timeoutMs = 10000,
): Promise<{ lat: number; lon: number }> {
  return new Promise((resolve, reject) => {
    if (typeof navigator === "undefined" || !navigator.geolocation) {
      reject(new Error("Location is not available on this system."));
      return;
    }
    navigator.geolocation.getCurrentPosition(
      (pos) => resolve({ lat: pos.coords.latitude, lon: pos.coords.longitude }),
      (err) => reject(new Error(err.message || "Location permission was denied.")),
      { timeout: timeoutMs, maximumAge: 60_000, enableHighAccuracy: false },
    );
  });
}

/** Best OS location for the Weather widget: precise coordinates from the OS,
 * then a reverse-geocoded city label (empty if that lookup fails — cosmetic).
 * Rejects only when the coordinates themselves are unavailable (permission
 * denied / timeout), so the auto-default path can leave the fields empty. */
export async function currentLocation(): Promise<{
  lat: number;
  lon: number;
  city: string;
}> {
  const { lat, lon } = await browserGeolocation();
  let city = "";
  try {
    city = await reverseGeocode(lat, lon);
  } catch {
    city = "";
  }
  return { lat, lon, city };
}

// --- Schedules page (CRUD + history) ---------------------------------

export function schedulesList(): Promise<Schedule[]> {
  return invoke("schedules_list");
}

/** Creates (new `id`) or updates (existing `id`) one schedule and rebuilds
 * the live cron job set. */
export function scheduleSave(schedule: Schedule): Promise<void> {
  return invoke("schedule_save", { schedule });
}

export function scheduleDelete(id: string): Promise<void> {
  return invoke("schedule_delete", { id });
}

/** Manually triggers one schedule now (same overlap/retry/history logic a
 * cron trigger would use) and resolves with the resulting history entry. */
export function scheduleRunNow(id: string): Promise<HistoryEntry> {
  return invoke("schedule_run_now", { id });
}

export function historyList(limit?: number): Promise<HistoryEntry[]> {
  return invoke("history_list", { limit });
}

/** Raw device fail codes (from the firmware's `{status:"fail", msg:"…"}`
 * body, surfaced by the Rust client) mapped to clear, human-readable text. */
const DEVICE_ERROR_MESSAGES: Record<string, string> = {
  NAME_TOO_LONG: "Device name is too long (16 characters max).",
  "Playlist not found": "That playlist no longer exists on the device.",
  "Bad args": "The device rejected the request (bad arguments).",
};

/** Turn a raw device fail code into a friendly sentence, leaving anything
 * unrecognized untouched. */
function humanizeDeviceError(msg: string): string {
  return DEVICE_ERROR_MESSAGES[msg.trim()] ?? msg;
}

/** Tauri command errors reject with whatever the Rust side returned as
 * `Err(String)`, which invoke() surfaces as a plain string (or occasionally
 * wraps) — normalize whatever shape shows up into a displayable message, and
 * translate known raw device fail codes into clear text. */
export function errorMessage(e: unknown): string {
  let raw: string;
  if (typeof e === "string") raw = e;
  else if (e instanceof Error) raw = e.message;
  else {
    try {
      raw = JSON.stringify(e);
    } catch {
      raw = String(e);
    }
  }
  return humanizeDeviceError(raw);
}

//! Tauri command surface for config persistence + Device page.
//!
//! Every device-control command re-reads `device.json`, builds a fresh
//! [`DeviceClient`] for the active device, and performs a single call —
//! there is no long-lived client cached across commands ("別過度
//! 設計"). Errors are converted to plain, user-displayable `String`s so the
//! frontend never has to pattern-match on Rust error enums.

use base64::Engine as _;
use tauri::{AppHandle, Manager};

use crate::config::{self, AppConfig};
use crate::device::{
    DeviceClient, DeviceInfo, DeviceSettingsUpdate, GalleryImage, GallerySummary, PlaylistSummary,
    ShowRequest,
};
use crate::render_service;
use crate::widgets::config::WidgetRenderConfig;

/// Default per-image duration (seconds) used by `show_gallery` — the device
/// protocol requires a duration for gallery slideshows
/// (`play_type=1`) but the Gallery page UI doesn't expose one yet.
const DEFAULT_SLIDESHOW_DURATION_SECS: u32 = 30;

/// Panel pixel size assumed when no device is reachable yet (EL133UF1's
/// portrait-native panel), so Preview still works before a device
/// is configured/awake. `pub(crate)` so the scheduler (`scheduler.rs`) can
/// reuse the exact same fallback when a scheduled push runs before a device
/// has ever answered `/deviceInfo`.
pub(crate) const DEFAULT_PANEL_WIDTH: u32 = 1200;
pub(crate) const DEFAULT_PANEL_HEIGHT: u32 = 1600;

/// Gallery every widget push lands in — v1 keeps this fixed (no
/// per-widget gallery picker yet); each widget kind still gets its own
/// upload-filename prefix (see `WidgetConfig::upload_prefix`). `pub(crate)`
/// so `scheduler.rs` pushes land in the same place as manual Widgets-page
/// pushes.
pub(crate) const WIDGET_GALLERY: &str = "default";

fn config_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve app data directory: {e}"))?;
    Ok(dir.join(config::CONFIG_FILE_NAME))
}

/// Normalize a user-entered address (`192.168.1.42`, `192.168.1.42:8080`,
/// `http://192.168.1.42`, or a MockDevice's `127.0.0.1:18080`) into the
/// `http://host[:port]` base URL [`DeviceClient`] expects.
fn normalize_base_url(input: &str) -> String {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

/// `pub(crate)` (not just a `commands.rs`-private helper) so `scheduler.rs`
/// can resolve the same active device a manual Widgets-page push would use.
pub(crate) fn active_client(app: &AppHandle) -> Result<DeviceClient, String> {
    let path = config_path(app)?;
    let cfg = config::load(&path).map_err(|e| e.to_string())?;
    let device = cfg
        .active_device()
        .ok_or_else(|| "no device configured yet — save the connection settings first".to_string())?;
    if device.lan_ip.trim().is_empty() {
        return Err("device LAN IP is empty — save the connection settings first".to_string());
    }
    Ok(DeviceClient::new(normalize_base_url(&device.lan_ip))
        .with_ble(device.ble_name.clone(), Some(device.name.clone())))
}

/// Resolve the [`DeviceClient`] a schedule should push to — the device the
/// schedule is bound to (`device_id`), *not* whichever device the UI happens
/// to have selected. An empty `device_id` (a legacy schedule from before
/// per-device scheduling) falls back to the active device. A non-empty id that
/// no longer matches any configured device (it was deleted) is a clear error;
/// `scheduler.rs` turns that into a skipped run rather than a retry loop.
pub(crate) fn client_for_device(app: &AppHandle, device_id: &str) -> Result<DeviceClient, String> {
    let path = config_path(app)?;
    let cfg = config::load(&path).map_err(|e| e.to_string())?;
    let device = cfg
        .device_for_schedule(device_id)
        .ok_or_else(|| "device no longer configured".to_string())?;
    if device.lan_ip.trim().is_empty() {
        return Err("device LAN IP is empty — save the connection settings first".to_string());
    }
    Ok(DeviceClient::new(normalize_base_url(&device.lan_ip))
        .with_ble(device.ble_name.clone(), Some(device.name.clone())))
}

/// The active device's opaque id, for keying the `last_push` store to the same
/// device a manual push (`push_image`/`push_widget`) targets. `None` when no
/// device is configured — recording is best-effort and simply skipped then.
pub(crate) fn active_device_id(app: &AppHandle) -> Option<String> {
    let path = config_path(app).ok()?;
    let cfg = config::load(&path).ok()?;
    cfg.active_device().map(|d| d.id.clone())
}

/// The id of the device a schedule bound to `device_id` actually pushes to
/// (resolving a legacy empty id to the active device) — so `last_push` is
/// recorded under the same id the Device-page hero later looks up.
pub(crate) fn resolved_schedule_device_id(app: &AppHandle, device_id: &str) -> Option<String> {
    let path = config_path(app).ok()?;
    let cfg = config::load(&path).ok()?;
    cfg.device_for_schedule(device_id).map(|d| d.id.clone())
}

#[tauri::command]
pub fn get_config(app: AppHandle) -> Result<AppConfig, String> {
    let path = config_path(&app)?;
    config::load(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_config(app: AppHandle, config: AppConfig) -> Result<(), String> {
    let path = config_path(&app)?;
    config::save(&path, &config).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn device_info(app: AppHandle) -> Result<DeviceInfo, String> {
    let client = active_client(&app)?;
    let info = client.info().await.map_err(|e| e.to_string())?;
    // Passively refresh the tray tooltip from info the UI already asked for
    // (the tray itself never polls or wakes the device).
    crate::tray::note_device_info(&app, &info);
    Ok(info)
}

#[tauri::command]
pub async fn device_wake(app: AppHandle) -> Result<(), String> {
    let client = active_client(&app)?;
    client.wake_if_needed().await.map_err(|e| e.to_string())
}

/// Scan BLE for Bloomin8-like peripherals whose advertised name contains
/// `hint` (empty -> "Bloomin8"), strongest signal first. Used by the
/// add-device flow to confirm the Canvas is reachable over Bluetooth. Never
/// errors — an empty list means "nothing found (asleep/out of range/no
/// permission)".
#[tauri::command]
pub async fn ble_scan(hint: String) -> Result<Vec<crate::device::wake::BleMatch>, String> {
    Ok(crate::device::wake::ble_scan(&hint).await)
}

#[tauri::command]
pub async fn device_sleep(app: AppHandle) -> Result<(), String> {
    let client = active_client(&app)?;
    client.sleep().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn device_reboot(app: AppHandle) -> Result<(), String> {
    let client = active_client(&app)?;
    client.reboot().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn device_clear_screen(app: AppHandle) -> Result<(), String> {
    let client = active_client(&app)?;
    client.clear_screen().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn device_set_settings(
    app: AppHandle,
    settings: DeviceSettingsUpdate,
) -> Result<(), String> {
    let client = active_client(&app)?;
    client.set_settings(&settings).await.map_err(|e| e.to_string())
}

// --- Gallery page ---------------------------------------------------

#[tauri::command]
pub async fn gallery_list(app: AppHandle) -> Result<Vec<GallerySummary>, String> {
    let client = active_client(&app)?;
    client.gallery_list().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn gallery_create(app: AppHandle, name: String) -> Result<(), String> {
    let client = active_client(&app)?;
    client.gallery_create(&name).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn gallery_delete(app: AppHandle, name: String) -> Result<(), String> {
    let client = active_client(&app)?;
    client.gallery_delete(&name).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn gallery_images(
    app: AppHandle,
    gallery: String,
    offset: u32,
    limit: u32,
) -> Result<Vec<GalleryImage>, String> {
    let client = active_client(&app)?;
    client
        .gallery_images(&gallery, offset, limit)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn image_delete(app: AppHandle, gallery: String, image: String) -> Result<(), String> {
    let client = active_client(&app)?;
    client.image_delete(&gallery, &image).await.map_err(|e| e.to_string())
}

/// `POST /show` with `play_type=0` for a single image already present in
/// `gallery` — builds the device-path `image` field (`/gallerys/<gallery>/<name>`)
/// that the firmware expects (`show-image`).
#[tauri::command]
pub async fn show_image(app: AppHandle, gallery: String, image: String) -> Result<(), String> {
    let client = active_client(&app)?;
    client
        .show(&ShowRequest::Image {
            image: format!("/gallerys/{gallery}/{image}"),
        })
        .await
        .map_err(|e| e.to_string())
}

/// `POST /show` with `play_type=1` — start a gallery slideshow.
#[tauri::command]
pub async fn show_gallery(app: AppHandle, gallery: String) -> Result<(), String> {
    let client = active_client(&app)?;
    client
        .show(&ShowRequest::Gallery {
            gallery,
            duration: DEFAULT_SLIDESHOW_DURATION_SECS,
        })
        .await
        .map_err(|e| e.to_string())
}

/// `POST /show` with `play_type=2` — start a playlist.
#[tauri::command]
pub async fn show_playlist(app: AppHandle, playlist: String) -> Result<(), String> {
    let client = active_client(&app)?;
    client
        .show(&ShowRequest::Playlist { playlist })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn show_next(app: AppHandle) -> Result<(), String> {
    let client = active_client(&app)?;
    client.show_next().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn playlist_list(app: AppHandle) -> Result<Vec<PlaylistSummary>, String> {
    let client = active_client(&app)?;
    client.playlist_list().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn playlist_get(app: AppHandle, name: String) -> Result<serde_json::Value, String> {
    let client = active_client(&app)?;
    client.playlist_get(&name).await.map_err(|e| e.to_string())
}

/// `PUT /playlist` — create/overwrite a playlist. `body` is the raw JSON
/// document (e.g. `{"name": ..., "type": ..., "list": [...]}`); `name` is
/// stamped in by [`DeviceClient::playlist_put`] if the body doesn't already
/// carry one.
#[tauri::command]
pub async fn playlist_save(
    app: AppHandle,
    name: String,
    body: serde_json::Value,
) -> Result<(), String> {
    let client = active_client(&app)?;
    client.playlist_put(&name, body).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn playlist_delete(app: AppHandle, name: String) -> Result<(), String> {
    let client = active_client(&app)?;
    client.playlist_delete(&name).await.map_err(|e| e.to_string())
}

// --- Widgets page (preview + push) ----------------------------------

/// Directory used to cache widget-fetched assets (coin icons, Met Museum
/// artwork) — a subdirectory of the app data dir, kept separate from
/// `device.json` and any future `schedules.json`/`history.jsonl`.
pub(crate) fn widget_cache_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve app data directory: {e}"))?;
    Ok(dir.join("widget-cache"))
}

/// Reject names that could escape the cache dir or the device's gallery path.
/// Device filenames are plain `<...>.jpg` — no separators.
fn is_safe_component(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains('/')
        && !s.contains('\\')
        && !s.contains("..")
}

/// Remove all local data tied to a device being deleted: its on-disk image
/// cache (`image-cache/<device_id>/`) and its last-push record. Schedules are
/// removed separately (via the scheduler) so their cron jobs get torn down.
/// Best-effort: missing files are not an error.
#[tauri::command]
pub fn purge_device_data(app: AppHandle, device_id: String) -> Result<(), String> {
    if !is_safe_component(&device_id) {
        return Err("invalid device id".to_string());
    }
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::remove_dir_all(dir.join("image-cache").join(&device_id));
    }
    crate::last_push::remove(&app, &device_id);
    Ok(())
}

/// Fetch a device image as a `data:image/jpeg;base64,...` URL, cached on disk
/// forever (filenames are never reused, so `(gallery, name)`
/// bytes are immutable). One full-resolution copy is cached per image; the
/// frontend sizes it down with CSS, so the device is hit at most once per
/// image regardless of where it's displayed.
#[tauri::command]
pub async fn fetch_image(app: AppHandle, gallery: String, name: String) -> Result<String, String> {
    if !is_safe_component(&gallery) || !is_safe_component(&name) {
        return Err("invalid gallery or image name".to_string());
    }

    // Segment the disk cache by active device id so two devices with the same
    // gallery/filename don't overwrite each other. Falls back to a fixed
    // "default" bucket when no device is configured or the id isn't path-safe.
    let cfg_path = config_path(&app)?;
    let cfg = config::load(&cfg_path).map_err(|e| e.to_string())?;
    let device_id = cfg
        .active_device()
        .map(|d| d.id.as_str())
        .filter(|id| is_safe_component(id))
        .unwrap_or("default")
        .to_string();

    let cache_file = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve app data directory: {e}"))?
        .join("image-cache")
        .join(&device_id)
        .join(&gallery)
        .join(&name);

    // Cache hit: serve from disk, no network.
    if let Ok(bytes) = std::fs::read(&cache_file) {
        return Ok(to_jpeg_data_url(&bytes));
    }

    // Miss: fetch the full image from the device and cache it.
    let client = active_client(&app)?;
    let bytes = client
        .image_bytes(&gallery, &name)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(dir) = cache_file.parent() {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&cache_file, &bytes);
    }
    Ok(to_jpeg_data_url(&bytes))
}

fn to_jpeg_data_url(bytes: &[u8]) -> String {
    format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

// --- Image push (upload an arbitrary photo with display settings) -----------
//
// The device API has NO fit/fill/orientation/border parameters (verified
// against the official OpenAPI) — it only stores/shows pre-sized JPEGs. So the
// display settings the official app offers are applied *here*, client-side,
// producing an exact panel-sized JPEG before upload (same approach as the
// widget pipeline and the Python reference `eink_render.py`).

const IMAGE_JPEG_QUALITY: u8 = 92;

/// Decode a `data:<mime>;base64,<...>` URL (from a file `<input>` or a cached
/// gallery image) into raw bytes. `pub(crate)` so `scheduler.rs` can decode an
/// image-schedule source the same way a manual push does.
pub(crate) fn decode_data_url(data_url: &str) -> Result<Vec<u8>, String> {
    let b64 = data_url.rsplit(',').next().unwrap_or("");
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("could not decode image data: {e}"))
}

/// Fit a source image onto the panel per the chosen display settings and
/// return a panel-sized (`panel_width x panel_height`) JPEG.
///
/// - `landscape`: mount the frame sideways — compose on a swapped-size canvas,
///   then rotate the bitmap back to portrait-native (like the widget pipeline).
/// - `mode`: `"fill"` covers the panel (center-crop overflow), `"fit"` shows
///   the whole image padded with the border color, `"auto"` fills a
///   portrait-ish source and fits (with border) a landscape-ish one.
/// - `border_white`: padding color for `fit`/`auto`-fit (white vs black).
///
/// `pub(crate)` so `scheduler.rs`'s image schedules produce a panel-ready JPEG
/// exactly the way a manual push does.
pub(crate) fn process_push_image(
    bytes: &[u8],
    panel_width: u32,
    panel_height: u32,
    landscape: bool,
    rotate_cw: bool,
    mode: &str,
    border_white: bool,
) -> Result<Vec<u8>, String> {
    use image::imageops::{self, FilterType};

    let img = image::load_from_memory(bytes)
        .map_err(|e| format!("unsupported or corrupt image: {e}"))?
        .to_rgb8();
    let (iw, ih) = (img.width().max(1), img.height().max(1));
    // Compose on the panel canvas (swapped for landscape mounting).
    let (cw, ch) = if landscape {
        (panel_height, panel_width)
    } else {
        (panel_width, panel_height)
    };

    let fill = match mode {
        "fill" => true,
        "fit" => false,
        _ => ih >= iw, // auto: portrait source fills, landscape source fits
    };

    let composed: image::RgbImage = if fill {
        // Cover: scale so the image fully covers the canvas, then center-crop.
        let scale = (cw as f64 / iw as f64).max(ch as f64 / ih as f64);
        let rw = ((iw as f64 * scale).round() as u32).max(cw);
        let rh = ((ih as f64 * scale).round() as u32).max(ch);
        let resized = imageops::resize(&img, rw, rh, FilterType::Lanczos3);
        imageops::crop_imm(&resized, (rw - cw) / 2, (rh - ch) / 2, cw, ch).to_image()
    } else {
        // Contain: scale to fit inside the canvas, pad with the border color.
        let border = if border_white {
            image::Rgb([255u8, 255, 255])
        } else {
            image::Rgb([0u8, 0, 0])
        };
        let mut canvas = image::RgbImage::from_pixel(cw, ch, border);
        let scale = (cw as f64 / iw as f64).min(ch as f64 / ih as f64);
        let rw = ((iw as f64 * scale).round() as u32).clamp(1, cw);
        let rh = ((ih as f64 * scale).round() as u32).clamp(1, ch);
        let resized = imageops::resize(&img, rw, rh, FilterType::Lanczos3);
        imageops::overlay(
            &mut canvas,
            &resized,
            ((cw - rw) / 2) as i64,
            ((ch - rh) / 2) as i64,
        );
        canvas
    };

    let final_img = if landscape {
        if rotate_cw {
            imageops::rotate90(&composed)
        } else {
            imageops::rotate270(&composed)
        }
    } else {
        composed
    };
    debug_assert_eq!((final_img.width(), final_img.height()), (panel_width, panel_height));

    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, IMAGE_JPEG_QUALITY)
        .encode_image(&final_img)
        .map_err(|e| format!("failed to encode JPEG: {e}"))?;
    Ok(jpeg)
}

/// Panel size from the active device (falls back to the default when it can't
/// be reached, so preview still works offline).
async fn active_panel_size(app: &AppHandle) -> (u32, u32) {
    match active_client(app) {
        Ok(client) => match client.info().await {
            Ok(info) => (info.width, info.height),
            Err(_) => (DEFAULT_PANEL_WIDTH, DEFAULT_PANEL_HEIGHT),
        },
        Err(_) => (DEFAULT_PANEL_WIDTH, DEFAULT_PANEL_HEIGHT),
    }
}

/// Process `source` (a data URL) per the display settings and return the
/// panel-ready JPEG as a data URL — for the push dialog's live preview.
/// Never touches the device except to read its panel size.
#[tauri::command]
pub async fn preview_image(
    app: AppHandle,
    source: String,
    orientation: String,
    rotate: String,
    mode: String,
    border: String,
) -> Result<String, String> {
    let (pw, ph) = active_panel_size(&app).await;
    let bytes = decode_data_url(&source)?;
    let jpeg = process_push_image(
        &bytes,
        pw,
        ph,
        orientation == "landscape",
        rotate != "ccw",
        &mode,
        border == "white",
    )?;
    Ok(to_jpeg_data_url(&jpeg))
}

/// Process `source` per the display settings and push it to the device
/// (wake-if-needed -> upload under a fresh `photo_<ts>.jpg` -> show + verify).
/// Resolves with the filename now displayed.
#[tauri::command]
pub async fn push_image(
    app: AppHandle,
    source: String,
    orientation: String,
    rotate: String,
    mode: String,
    border: String,
) -> Result<String, String> {
    let client = active_client(&app)?;
    let (pw, ph) = match client.info().await {
        Ok(info) => (info.width, info.height),
        Err(_) => (DEFAULT_PANEL_WIDTH, DEFAULT_PANEL_HEIGHT),
    };
    let bytes = decode_data_url(&source)?;
    let jpeg = process_push_image(
        &bytes,
        pw,
        ph,
        orientation == "landscape",
        rotate != "ccw",
        &mode,
        border == "white",
    )?;
    client.wake_if_needed().await.map_err(|e| e.to_string())?;
    let now = chrono::Local::now().naive_local();
    let timestamp = now.format("%Y%m%d_%H%M%S").to_string();
    let outcome = client
        .upload_and_show(jpeg, "photo", WIDGET_GALLERY, &timestamp)
        .await
        .map_err(|e| e.to_string())?;
    // Remember this push so the Device-page hero can show a landscape image
    // upright (best-effort — a failure here never fails the push).
    if let Some(device_id) = active_device_id(&app) {
        let orientation = if orientation == "landscape" {
            "landscape"
        } else {
            "portrait"
        };
        crate::last_push::record(&app, &device_id, &outcome.filename, orientation);
    }
    Ok(outcome.filename)
}

/// Approximate location resolved from the machine's public IP.
#[derive(serde::Serialize)]
pub struct GeoLocation {
    pub lat: f64,
    pub lon: f64,
    pub city: String,
}

/// Look up an approximate lat/lon/city from the machine's public IP via the
/// free, key-less ip-api.com service — used by the Weather widget's "Use my
/// location" button to prefill coordinates. Best-effort: any failure returns
/// a displayable error the UI shows without falling over.
#[tauri::command]
pub async fn ip_geolocation() -> Result<GeoLocation, String> {
    #[derive(serde::Deserialize)]
    struct IpApi {
        status: String,
        message: Option<String>,
        lat: Option<f64>,
        lon: Option<f64>,
        city: Option<String>,
    }
    let resp = reqwest::Client::new()
        .get("http://ip-api.com/json/?fields=status,message,lat,lon,city")
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
        .map_err(|e| format!("location lookup failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("location lookup failed: {e}"))?;
    let body: IpApi = resp
        .json()
        .await
        .map_err(|e| format!("could not parse location response: {e}"))?;
    if body.status != "success" {
        return Err(body
            .message
            .unwrap_or_else(|| "location lookup failed".to_string()));
    }
    match (body.lat, body.lon) {
        (Some(lat), Some(lon)) => Ok(GeoLocation {
            lat,
            lon,
            city: body.city.unwrap_or_default(),
        }),
        _ => Err("location response was missing coordinates".to_string()),
    }
}

/// Reverse-geocode a lat/lon into a human city label via the free, key-less
/// BigDataCloud `reverse-geocode-client` endpoint. The label is purely
/// cosmetic — the weather fetch itself needs only lat/lon — so callers treat
/// any error as "no label". Returns an empty string when the service has no
/// name for the point.
#[tauri::command]
pub async fn reverse_geocode(lat: f64, lon: f64) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct Bdc {
        city: Option<String>,
        locality: Option<String>,
        #[serde(rename = "principalSubdivision")]
        principal_subdivision: Option<String>,
    }
    let resp = reqwest::Client::new()
        .get("https://api.bigdatacloud.net/data/reverse-geocode-client")
        .query(&[
            ("latitude", lat.to_string()),
            ("longitude", lon.to_string()),
            ("localityLanguage", "en".to_string()),
        ])
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
        .map_err(|e| format!("reverse geocode failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("reverse geocode failed: {e}"))?;
    let body: Bdc = resp
        .json()
        .await
        .map_err(|e| format!("could not parse reverse geocode response: {e}"))?;
    // Prefer the most specific name available (city -> locality -> subdivision).
    let name = [body.city, body.locality, body.principal_subdivision]
        .into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty())
        .unwrap_or_default();
    Ok(name)
}

/// Renders `req` and returns it as a `data:image/jpeg;base64,...` URL the
/// frontend can drop straight into an `<img>` for the Widgets page's Preview
/// button. Sizes to the active device's real panel dimensions when
/// reachable, otherwise a documented default — Preview never touches the
/// device itself.
#[tauri::command]
pub async fn preview_widget(app: AppHandle, req: WidgetRenderConfig) -> Result<String, String> {
    let (panel_width, panel_height) = match active_client(&app) {
        Ok(client) => match client.info().await {
            Ok(info) => (info.width, info.height),
            Err(_) => (DEFAULT_PANEL_WIDTH, DEFAULT_PANEL_HEIGHT),
        },
        Err(_) => (DEFAULT_PANEL_WIDTH, DEFAULT_PANEL_HEIGHT),
    };
    let cache_dir = widget_cache_dir(&app)?;
    let now = chrono::Local::now().naive_local();

    // Preview shows the upright render (rotate_to_panel = false) so landscape
    // widgets preview the right way up, not the sideways panel-native bitmap.
    let jpeg = render_service::render_widget_config(
        &app,
        &req,
        panel_width,
        panel_height,
        &cache_dir,
        now,
        false,
    )
    .await?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
}

/// Renders `req` and pushes it to the active device (wake-if-needed ->
/// render -> `upload_and_show`), returning the filename now displayed on the
/// panel. This is a thin Tauri-command wrapper around
/// `render_service::push_widget_config` — the scheduler calls that
/// function directly instead of going through this command.
#[tauri::command]
pub async fn push_widget(app: AppHandle, req: WidgetRenderConfig) -> Result<String, String> {
    let client = active_client(&app)?;
    let (panel_width, panel_height) = match client.info().await {
        Ok(info) => (info.width, info.height),
        Err(_) => (DEFAULT_PANEL_WIDTH, DEFAULT_PANEL_HEIGHT),
    };
    let cache_dir = widget_cache_dir(&app)?;
    let now = chrono::Local::now().naive_local();

    let outcome = render_service::push_widget_config(
        &app,
        &client,
        &req,
        panel_width,
        panel_height,
        &cache_dir,
        WIDGET_GALLERY,
        now,
    )
    .await?;
    // Remember this push's orientation so the Device-page hero can render a
    // landscape widget upright (best-effort — never fails the push).
    if let Some(device_id) = active_device_id(&app) {
        crate::last_push::record(
            &app,
            &device_id,
            &outcome.filename,
            orientation_str(req.orientation),
        );
    }
    Ok(outcome.filename)
}

/// `PanelOrientation` as the `"portrait"`/`"landscape"` string the `last_push`
/// store (and the frontend) use.
pub(crate) fn orientation_str(orientation: crate::widgets::config::PanelOrientation) -> &'static str {
    match orientation {
        crate::widgets::config::PanelOrientation::Landscape => "landscape",
        crate::widgets::config::PanelOrientation::Portrait => "portrait",
    }
}

// --- Schedules page (CRUD + history) --------------------------------

fn schedules_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve app data directory: {e}"))?;
    Ok(dir.join(crate::scheduler::SCHEDULES_FILE_NAME))
}

fn history_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve app data directory: {e}"))?;
    Ok(dir.join(crate::scheduler::HISTORY_FILE_NAME))
}

/// Number of history entries the Schedules page fetches by default.
const DEFAULT_HISTORY_LIMIT: usize = 200;

#[tauri::command]
pub fn schedules_list(app: AppHandle) -> Result<Vec<crate::scheduler::Schedule>, String> {
    let path = schedules_path(&app)?;
    crate::scheduler::load_schedules(&path).map_err(|e| e.to_string())
}

/// Creates (if `schedule.id` is new) or updates (if it matches an existing
/// entry) one schedule, persists `schedules.json`, then rebuilds the live
/// cron job set so the change takes effect immediately.
#[tauri::command]
pub async fn schedule_save(
    app: AppHandle,
    scheduler: tauri::State<'_, std::sync::Arc<crate::scheduler::SchedulerManager>>,
    schedule: crate::scheduler::Schedule,
) -> Result<(), String> {
    let path = schedules_path(&app)?;
    let mut list = crate::scheduler::load_schedules(&path).map_err(|e| e.to_string())?;
    match list.iter_mut().find(|s| s.id == schedule.id) {
        Some(existing) => *existing = schedule,
        None => list.push(schedule),
    }
    crate::scheduler::save_schedules(&path, &list).map_err(|e| e.to_string())?;
    scheduler.reload(&app).await?;
    // Keep the tray's "Refresh <name> now" entries in sync with
    // schedules.json (no-op if the tray hasn't been built, e.g. in the
    // windowless debug modes).
    crate::tray::rebuild_menu(&app);
    Ok(())
}

#[tauri::command]
pub async fn schedule_delete(
    app: AppHandle,
    scheduler: tauri::State<'_, std::sync::Arc<crate::scheduler::SchedulerManager>>,
    id: String,
) -> Result<(), String> {
    let path = schedules_path(&app)?;
    let mut list = crate::scheduler::load_schedules(&path).map_err(|e| e.to_string())?;
    list.retain(|s| s.id != id);
    crate::scheduler::save_schedules(&path, &list).map_err(|e| e.to_string())?;
    scheduler.reload(&app).await?;
    crate::tray::rebuild_menu(&app);
    Ok(())
}

/// Manually triggers one schedule right now, going through the same
/// overlap-prevention/retry-once/history logic a cron trigger would.
#[tauri::command]
pub async fn schedule_run_now(
    app: AppHandle,
    scheduler: tauri::State<'_, std::sync::Arc<crate::scheduler::SchedulerManager>>,
    id: String,
) -> Result<crate::scheduler::HistoryEntry, String> {
    scheduler.run_now(&app, &id).await
}

#[tauri::command]
pub fn history_list(
    app: AppHandle,
    limit: Option<usize>,
) -> Result<Vec<crate::scheduler::HistoryEntry>, String> {
    let path = history_path(&app)?;
    crate::scheduler::read_history(&path, limit.unwrap_or(DEFAULT_HISTORY_LIMIT)).map_err(|e| e.to_string())
}

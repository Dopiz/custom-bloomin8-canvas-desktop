use std::future::Future;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use super::error::DeviceError;
use super::types::{
    DeviceInfo, DeviceSettingsUpdate, DeviceState, GalleryImage, GallerySummary, PlaylistSummary,
    ShowRequest, UploadAndShowResult,
};
use super::wake;

/// Default timeout for `GET /deviceInfo`. Kept short so a
/// sleeping/unreachable device fails fast instead of blocking a caller that
/// is trying to decide whether to trigger a BLE wake.
const DEFAULT_INFO_TIMEOUT: Duration = Duration::from_secs(3);
/// Default timeout for `GET /state`.
const DEFAULT_STATE_TIMEOUT: Duration = Duration::from_secs(5);
/// Default timeout for simple power/settings actions.
const DEFAULT_ACTION_TIMEOUT: Duration = Duration::from_secs(15);
/// Default timeout for `POST /upload` (bigger payload than other calls).
const DEFAULT_UPLOAD_TIMEOUT: Duration = Duration::from_secs(30);
/// Default budget for the post-upload `wait_ready` poll (mirrors the
/// Python reference's `wait_ready(timeout=60, interval=2)`).
const DEFAULT_WAIT_READY_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_WAIT_READY_INTERVAL: Duration = Duration::from_secs(2);
/// Page size used internally by `cleanup` when listing a gallery's images.
const CLEANUP_LIST_LIMIT: u32 = 200;
/// Total budget for the post-wake-pulse `/deviceInfo` poll
/// (poll up to ~30-45s).
const DEFAULT_WAKE_BUDGET: Duration = Duration::from_secs(45);
/// Interval between poll attempts while waiting for the device to wake.
const DEFAULT_WAKE_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Thin async HTTP client for the Bloomin8 Canvas LAN protocol.
///
/// Ports `~/.claude/skills/bloomin8-canvas/scripts/client.py`'s semantics.
/// This slice only covers the "core" endpoints (info/state/wait_ready/power/
/// settings); upload, gallery and playlist calls are added in later slices.
#[derive(Debug, Clone)]
pub struct DeviceClient {
    base_url: String,
    http: Client,
    info_timeout: Duration,
    state_timeout: Duration,
    action_timeout: Duration,
    /// User's BLE match hint (substring) for this device; `None` -> the default
    /// "Bloomin8" hint. Used only by [`Self::wake_if_needed`].
    ble_hint: Option<String>,
    /// The device's stored real name, used to disambiguate when several
    /// peripherals match the hint. `None` when not yet known.
    ble_name: Option<String>,
}

impl DeviceClient {
    /// `base_url` is the device's HTTP origin, e.g. `http://192.168.1.42` (no
    /// trailing slash), or a MockDevice's `http://127.0.0.1:<port>`.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: Client::new(),
            info_timeout: DEFAULT_INFO_TIMEOUT,
            state_timeout: DEFAULT_STATE_TIMEOUT,
            action_timeout: DEFAULT_ACTION_TIMEOUT,
            ble_hint: None,
            ble_name: None,
        }
    }

    /// Override the default per-request timeouts. Exposed so tests can use
    /// short timeouts instead of waiting on the real ~3s/5s/15s defaults.
    pub fn with_timeouts(mut self, info: Duration, state: Duration, action: Duration) -> Self {
        self.info_timeout = info;
        self.state_timeout = state;
        self.action_timeout = action;
        self
    }

    /// Attach this device's BLE wake settings — the user's match `hint`
    /// (substring; empty falls back to the default at wake time) and the stored
    /// real `name` (for disambiguation). Threaded through by `commands.rs` when
    /// it builds the active-device client so [`Self::wake_if_needed`] targets
    /// the right peripheral.
    pub fn with_ble(mut self, hint: impl Into<String>, name: Option<String>) -> Self {
        let hint = hint.into();
        self.ble_hint = if hint.trim().is_empty() {
            None
        } else {
            Some(hint)
        };
        self.ble_name = name.filter(|n| !n.trim().is_empty());
        self
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// `GET /deviceInfo` — width, height, battery, current image, gallery,
    /// `max_idle`, etc.
    pub async fn info(&self) -> Result<DeviceInfo, DeviceError> {
        let resp = self
            .http
            .get(self.url("/deviceInfo"))
            .timeout(self.info_timeout)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json::<DeviceInfo>().await?)
    }

    /// `GET /state` — `{status, msg}`; `status == 100` means Ready.
    pub async fn state(&self) -> Result<DeviceState, DeviceError> {
        let resp = self
            .http
            .get(self.url("/state"))
            .timeout(self.state_timeout)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json::<DeviceState>().await?)
    }

    /// Poll `/state` until `status == 100` (Ready) or `timeout` elapses.
    ///
    /// Transient errors while polling (e.g. the device briefly unreachable
    /// mid-transition) are swallowed and retried rather than treated as
    /// fatal, mirroring `wait_ready` in the Python reference client.
    pub async fn wait_ready(
        &self,
        timeout: Duration,
        interval: Duration,
    ) -> Result<DeviceState, DeviceError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut last: Option<DeviceState> = None;

        loop {
            if let Ok(state) = self.state().await {
                if state.is_ready() {
                    return Ok(state);
                }
                last = Some(state);
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(DeviceError::NotReady(timeout, last));
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// `GET /whistle` — keep-alive ping that postpones sleep.
    pub async fn whistle(&self) -> Result<(), DeviceError> {
        self.http
            .get(self.url("/whistle"))
            .timeout(self.action_timeout)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// `POST /sleep` — put the device to sleep.
    pub async fn sleep(&self) -> Result<(), DeviceError> {
        self.http
            .post(self.url("/sleep"))
            .timeout(self.action_timeout)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// `POST /reboot`.
    pub async fn reboot(&self) -> Result<(), DeviceError> {
        self.http
            .post(self.url("/reboot"))
            .timeout(self.action_timeout)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// `POST /clearScreen` — clear the panel to white.
    pub async fn clear_screen(&self) -> Result<(), DeviceError> {
        self.http
            .post(self.url("/clearScreen"))
            .timeout(self.action_timeout)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// `POST /settings` — update any subset of `name`, `sleep_duration`,
    /// `max_idle`, `idx_wake_sens`. Surfaces the device's own error message
    /// (e.g. `NAME_TOO_LONG` when the name exceeds 16 chars) instead of a bare
    /// HTTP 500.
    pub async fn set_settings(&self, settings: &DeviceSettingsUpdate) -> Result<(), DeviceError> {
        let resp = self
            .http
            .post(self.url("/settings"))
            .timeout(self.action_timeout)
            .json(settings)
            .send()
            .await?;
        ensure_device_ok(resp).await?;
        Ok(())
    }

    /// `POST /upload` — multipart field `image`; `filename`/`gallery`/
    /// `show_now` are query params (`client.py::upload_and_show`).
    /// Does not itself verify the display took effect — see
    /// [`Self::upload_and_show`] for the full, verified flow.
    pub async fn upload(
        &self,
        bytes: Vec<u8>,
        filename: &str,
        gallery: &str,
        show_now: bool,
    ) -> Result<(), DeviceError> {
        // The `image` part must carry a filename + `image/jpeg` mime, matching
        // the reference client (`files={"image": open(...)}`): firmware 1.8.35
        // treats a part without file metadata as a non-file field and saves the
        // image without ever displaying it, so `/state` never leaves idle (0)
        // and the show-verification times out. (Verified against a real device.)
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str("image/jpeg")
            .map_err(DeviceError::Request)?;
        let form = reqwest::multipart::Form::new().part("image", part);

        self.http
            .post(self.url("/upload"))
            .timeout(DEFAULT_UPLOAD_TIMEOUT)
            .query(&[
                ("filename", filename),
                ("gallery", gallery),
                ("show_now", if show_now { "1" } else { "0" }),
            ])
            .multipart(form)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Upload `bytes` under a fresh `<prefix>_<timestamp>.jpg` filename,
    /// display it immediately, and verify the panel actually picked it up.
    ///
    /// `timestamp` is supplied by the caller (e.g. formatted from an
    /// injected clock) rather than computed here, so this function's core
    /// logic never touches `SystemTime::now()` directly and stays
    /// deterministic under test.
    ///
    /// The firmware caches a rendered image by filename —
    /// reusing a name (even after deleting it) can silently redisplay stale
    /// content while `/state` still reports Ready. So after the upload
    /// reaches Ready, we re-fetch `/deviceInfo` and require `.image` to end
    /// with the new filename; a mismatch is reported as
    /// [`DeviceError::DisplayVerificationFailed`] rather than treated as
    /// success.
    pub async fn upload_and_show(
        &self,
        bytes: Vec<u8>,
        prefix: &str,
        gallery: &str,
        timestamp: &str,
    ) -> Result<UploadAndShowResult, DeviceError> {
        let filename = format!("{prefix}_{timestamp}.jpg");

        self.upload(bytes, &filename, gallery, true).await?;
        self.wait_ready(DEFAULT_WAIT_READY_TIMEOUT, DEFAULT_WAIT_READY_INTERVAL)
            .await?;

        let info = self.info().await?;
        let shown = info.image.clone().unwrap_or_default();
        if !shown.ends_with(&filename) {
            return Err(DeviceError::DisplayVerificationFailed {
                expected: filename,
                actual: info.image,
            });
        }

        Ok(UploadAndShowResult {
            filename,
            gallery: gallery.to_string(),
        })
    }

    /// `GET /gallery?gallery_name=&offset=&limit=` — paginated listing of a
    /// gallery's images.
    pub async fn gallery_images(
        &self,
        gallery: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<GalleryImage>, DeviceError> {
        #[derive(Debug, Deserialize)]
        struct GalleryImagesResponse {
            #[serde(default)]
            data: Vec<GalleryImage>,
        }

        let resp = self
            .http
            .get(self.url("/gallery"))
            .timeout(self.action_timeout)
            .query(&[
                ("gallery_name", gallery.to_string()),
                ("offset", offset.to_string()),
                ("limit", limit.to_string()),
            ])
            .send()
            .await?
            .error_for_status()?;
        let parsed: GalleryImagesResponse = resp.json().await?;
        Ok(parsed.data)
    }

    /// `GET /gallerys/<gallery>/<name>` — the raw stored JPEG bytes. The
    /// device serves the full-resolution image at its `deviceInfo.image` path
    /// (there is no separate thumbnail endpoint), so callers that want a
    /// thumbnail downscale client-side. Filenames are never reused
    /// so `(gallery, name)` bytes are immutable and safe to
    /// cache forever.
    pub async fn image_bytes(&self, gallery: &str, name: &str) -> Result<Vec<u8>, DeviceError> {
        let resp = self
            .http
            .get(self.url(&format!("/gallerys/{gallery}/{name}")))
            .timeout(self.action_timeout)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// `POST /image/delete?image=<name>&gallery=<g>`.
    pub async fn image_delete(&self, gallery: &str, filename: &str) -> Result<(), DeviceError> {
        self.http
            .post(self.url("/image/delete"))
            .timeout(self.action_timeout)
            .query(&[("image", filename), ("gallery", gallery)])
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Delete every `<prefix>_*` image in `gallery` except `keep`.
    ///
    /// Refuses outright (without listing or deleting
    /// anything) if `keep` is empty/`None` — a failed upload piped straight
    /// into cleanup would otherwise wipe every same-prefix image, including
    /// whatever is currently displayed.
    pub async fn cleanup(
        &self,
        prefix: &str,
        keep: Option<&str>,
        gallery: &str,
    ) -> Result<Vec<String>, DeviceError> {
        let keep = keep.unwrap_or("");
        if keep.trim().is_empty() {
            return Err(DeviceError::CleanupRefused);
        }

        let images = self
            .gallery_images(gallery, 0, CLEANUP_LIST_LIMIT)
            .await?;
        let mut deleted = Vec::new();
        for image in images {
            if image.name.starts_with(prefix) && image.name != keep {
                self.image_delete(gallery, &image.name).await?;
                deleted.push(image.name);
            }
        }
        Ok(deleted)
    }

    /// Ensure the device is reachable, waking it via BLE if needed
    /// Tries `info()` first (using this client's usual
    /// `info_timeout`); if that fails, triggers the real BLE wake pulse and
    /// polls `info()` for up to 45s.
    pub async fn wake_if_needed(&self) -> Result<(), DeviceError> {
        let hint = self
            .ble_hint
            .clone()
            .unwrap_or_else(|| wake::DEFAULT_BLE_NAME.to_string());
        let name = self.ble_name.clone();
        self.wake_if_needed_with(
            move |hint| async move {
                // The discovered real name is persisted separately via
                // `/deviceInfo` (a more authoritative source), so it's
                // discarded here.
                let _ = wake::ble_wake_pulse(&hint, name.as_deref()).await;
            },
            hint,
            DEFAULT_WAKE_POLL_INTERVAL,
            DEFAULT_WAKE_BUDGET,
        )
        .await
    }

    /// Same orchestration as [`Self::wake_if_needed`], but with the BLE
    /// wake implementation and poll timing injected — used by tests to
    /// substitute a stub waker (and short poll interval/budget) instead of
    /// performing real BLE scanning.
    ///
    /// `waker` is a best-effort pulse, exactly like the production BLE
    /// path: its return value (`()`) carries no success/failure signal.
    /// Whether the device actually woke up is decided entirely by whether
    /// `info()` starts succeeding again.
    pub async fn wake_if_needed_with<F, Fut>(
        &self,
        waker: F,
        ble_name: String,
        poll_interval: Duration,
        wake_budget: Duration,
    ) -> Result<(), DeviceError>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = ()>,
    {
        if self.info().await.is_ok() {
            return Ok(());
        }

        waker(ble_name).await;

        let deadline = tokio::time::Instant::now() + wake_budget;
        loop {
            if self.info().await.is_ok() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(DeviceError::WakeTimeout(wake_budget));
            }
            tokio::time::sleep(poll_interval).await;
        }
    }

    /// `GET /gallery/list` — every gallery on the device.
    ///
    /// Firmware 1.8.35 returns a bare top-level JSON array
    /// (`[{"name":"default"},{"name":"upstream"}]`), NOT a `{"data": [...]}`
    /// envelope — unlike the paginated `GET /gallery` (see [`Self::gallery_images`]).
    pub async fn gallery_list(&self) -> Result<Vec<GallerySummary>, DeviceError> {
        let resp = self
            .http
            .get(self.url("/gallery/list"))
            .timeout(self.action_timeout)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json::<Vec<GallerySummary>>().await?)
    }

    /// `PUT /gallery?name=` — create an empty gallery.
    pub async fn gallery_create(&self, name: &str) -> Result<(), DeviceError> {
        self.http
            .put(self.url("/gallery"))
            .timeout(self.action_timeout)
            .query(&[("name", name)])
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// `DELETE /gallery?name=` — delete a gallery and every image inside it.
    pub async fn gallery_delete(&self, name: &str) -> Result<(), DeviceError> {
        self.http
            .delete(self.url("/gallery"))
            .timeout(self.action_timeout)
            .query(&[("name", name)])
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// `POST /show` — display a single image, start a gallery slideshow, or
    /// start a playlist, depending on `request`'s `play_type`.
    pub async fn show(&self, request: &ShowRequest) -> Result<(), DeviceError> {
        self.http
            .post(self.url("/show"))
            .timeout(self.action_timeout)
            .json(&request.to_json())
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// `POST /showNext` — skip to the next item in the current queue.
    pub async fn show_next(&self) -> Result<(), DeviceError> {
        self.http
            .post(self.url("/showNext"))
            .timeout(self.action_timeout)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// `GET /playlist/list` — every playlist on the device.
    pub async fn playlist_list(&self) -> Result<Vec<PlaylistSummary>, DeviceError> {
        // Like `/gallery/list`, firmware 1.8.35 returns a bare top-level array
        // (`[]` when empty), not a `{"data": [...]}` envelope.
        let resp = self
            .http
            .get(self.url("/playlist/list"))
            .timeout(self.action_timeout)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json::<Vec<PlaylistSummary>>().await?)
    }

    /// `GET /playlist?name=` — a playlist's raw JSON content (structure is
    /// device-defined, e.g. `{"name", "type", "list": [...]}`), so this is
    /// returned as-is rather than through a fixed struct.
    pub async fn playlist_get(&self, name: &str) -> Result<serde_json::Value, DeviceError> {
        let resp = self
            .http
            .get(self.url("/playlist"))
            .timeout(self.action_timeout)
            .query(&[("name", name)])
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json::<serde_json::Value>().await?)
    }

    /// `PUT /playlist` — create/overwrite a playlist from a JSON body. `name`
    /// is stamped into the body if it doesn't already carry one, matching
    /// `client.py`'s `playlist-set` (whose JSON argument already includes
    /// `name`).
    pub async fn playlist_put(
        &self,
        name: &str,
        mut body: serde_json::Value,
    ) -> Result<(), DeviceError> {
        if let serde_json::Value::Object(ref mut map) = body {
            map.entry("name")
                .or_insert_with(|| serde_json::Value::String(name.to_string()));
        }

        self.http
            .put(self.url("/playlist"))
            .timeout(self.action_timeout)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// `DELETE /playlist?name=`.
    pub async fn playlist_delete(&self, name: &str) -> Result<(), DeviceError> {
        self.http
            .delete(self.url("/playlist"))
            .timeout(self.action_timeout)
            .query(&[("name", name)])
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

/// On a non-2xx response, surface the device's `{status:"fail", msg:"…"}`
/// message (e.g. `NAME_TOO_LONG`) as [`DeviceError::Device`] instead of a bare
/// HTTP status.
async fn ensure_device_ok(resp: reqwest::Response) -> Result<reqwest::Response, DeviceError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v.get("msg")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("device returned HTTP {}", status.as_u16()));
    Err(DeviceError::Device(msg))
}

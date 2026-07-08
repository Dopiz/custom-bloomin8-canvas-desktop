//! Widget render pipeline: offscreen (hidden) WebviewWindow capture.
//!
//! Flow: caller hands us fully-rendered widget HTML -> we create a hidden
//! WebviewWindow at the exact logical size -> the window loads `capture.html`
//! (app URL, so Tauri IPC works), pulls the HTML via `get_capture_payload`,
//! document.write()s it, waits for fonts/layout and signals readiness via
//! `notify_capture_ready` -> Rust captures the WKWebView with the native
//! `takeSnapshot` API (PNG bytes), asserts exact dimensions, optionally
//! rotates a landscape capture back to portrait-native, and re-encodes as
//! JPEG quality 92.
//!
//! Why native snapshot instead of the preferred html-to-image: WebKit taints
//! any canvas that has drawn an SVG containing
//! <foreignObject> (the core mechanism of html-to-image/html2canvas), so
//! toDataURL/getImageData throw SecurityError. On top of that, html-to-image's
//! toPng awaits img.decode(), which never resolves in a hidden WKWebView
//! window. Both are WebKit-level constraints, not fixable from JS.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::oneshot;

use crate::fixtures;

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);
const JPEG_QUALITY: u8 = 92;

static CAPTURE_SEQ: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Public pipeline API.
// ---------------------------------------------------------------------------

/// Which way the landscape bitmap is rotated back to portrait-native before
/// upload. The direction depends on how the user physically
/// mounted the frame — same contract as the Python client's `--rotate cw|ccw`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    /// Rotate the landscape capture 90° clockwise.
    Cw,
    /// Rotate the landscape capture 90° counter-clockwise.
    Ccw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Capture directly at `panel_width x panel_height`.
    Portrait,
    /// Capture at swapped dimensions (`panel_height x panel_width`), then
    /// rotate ±90° back to portrait-native.
    Landscape(Rotation),
}

/// Renders `html` in a hidden WebviewWindow and returns JPEG (q92) bytes at
/// exactly `panel_width x panel_height` (portrait-native panel pixels, from
/// `deviceInfo` — e.g. 1200x1600 on EL133UF1).
///
/// `Orientation::Landscape` captures at the swapped size. When
/// `rotate_to_panel` is true the landscape bitmap is rotated back to
/// portrait-native before encoding, so the result is upload-ready
/// (`panel_width x panel_height`). When false the natural, *upright* capture
/// is returned as-is — used by the Widgets Preview so a landscape widget is
/// shown the right way up (1600x1200) instead of the sideways portrait bitmap
/// the panel actually stores.
pub async fn render_widget(
    app: &AppHandle,
    html: String,
    orientation: Orientation,
    panel_width: u32,
    panel_height: u32,
    rotate_to_panel: bool,
) -> Result<Vec<u8>, String> {
    // Hold the activity guard across window teardown so the "last window
    // closed" exit request (see `lib.rs`) cannot kill the app mid-pipeline.
    let state = app.state::<CaptureState>();
    let _guard = state.hold_active();

    let (w, h) = match orientation {
        Orientation::Portrait => (panel_width, panel_height),
        Orientation::Landscape(_) => (panel_height, panel_width),
    };
    let rgb = capture_html(app, html, w, h).await?;
    let rgb = match (rotate_to_panel, orientation) {
        // Upright preview, or already-portrait: no rotation.
        (false, _) | (true, Orientation::Portrait) => rgb,
        (true, Orientation::Landscape(Rotation::Cw)) => image::imageops::rotate90(&rgb),
        (true, Orientation::Landscape(Rotation::Ccw)) => image::imageops::rotate270(&rgb),
    };
    if rotate_to_panel {
        debug_assert_eq!((rgb.width(), rgb.height()), (panel_width, panel_height));
    }

    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|e| format!("failed to encode JPEG: {e}"))?;
    Ok(jpeg)
}

// ---------------------------------------------------------------------------
// State shared between the capture driver and the IPC commands.
// ---------------------------------------------------------------------------

struct ReadySignal {
    inner_width: f64,
    inner_height: f64,
    device_pixel_ratio: f64,
}

struct Pending {
    html: String,
    tx: Option<oneshot::Sender<Result<ReadySignal, String>>>,
}

#[derive(Default)]
pub struct CaptureState {
    pending: Mutex<HashMap<String, Pending>>,
    /// Number of live captures/batches; while > 0, `lib.rs` prevents the
    /// window-close-triggered app exit (Tauri exits when the last window —
    /// here: a hidden capture window — is destroyed, which killed background
    /// multi-capture runs after the first image).
    active: AtomicUsize,
}

/// RAII guard: while at least one is alive, the app must not exit on
/// "last window closed".
pub struct CaptureActivityGuard<'a>(&'a AtomicUsize);

impl Drop for CaptureActivityGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl CaptureState {
    pub fn hold_active(&self) -> CaptureActivityGuard<'_> {
        self.active.fetch_add(1, Ordering::SeqCst);
        CaptureActivityGuard(&self.active)
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst) > 0
    }
}

// ---------------------------------------------------------------------------
// Capture driver.
// ---------------------------------------------------------------------------

/// Renders `html` at `width`x`height` logical pixels in a hidden
/// WebviewWindow and returns the decoded RGB capture, asserted to be exactly
/// `width`x`height`.
async fn capture_html(
    app: &AppHandle,
    html: String,
    width: u32,
    height: u32,
) -> Result<image::RgbImage, String> {
    let label = format!("capture-{}", CAPTURE_SEQ.fetch_add(1, Ordering::SeqCst));
    let (tx, rx) = oneshot::channel();

    let state = app.state::<CaptureState>();
    state
        .pending
        .lock()
        .unwrap()
        .insert(label.clone(), Pending { html, tx: Some(tx) });

    let window = build_capture_window(app, &label, width, height).map_err(|e| {
        app.state::<CaptureState>()
            .pending
            .lock()
            .unwrap()
            .remove(&label);
        format!("failed to create capture window: {e}")
    })?;

    // Wait for the page to signal "template loaded, fonts ready", then take a
    // native WKWebView snapshot. Tear down the window no matter what happened.
    let result = async {
        let ready = tokio::time::timeout(CAPTURE_TIMEOUT, rx)
            .await
            .map_err(|_| format!("capture timed out after {CAPTURE_TIMEOUT:?}"))?
            .map_err(|_| "capture channel dropped".to_string())??;
        // WKSnapshotConfiguration.snapshotWidth is in POINTS and the output
        // bitmap is points * backingScaleFactor, so divide by the window's
        // scale factor to land on exactly `width` pixels.
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let png = take_native_snapshot(&window, width as f64 / scale).await?;
        decode_exact(png, ready, width, height)
    }
    .await;

    let _ = window.destroy();
    app.state::<CaptureState>()
        .pending
        .lock()
        .unwrap()
        .remove(&label);

    result
}

/// Captures the webview content as PNG bytes using WKWebView's takeSnapshot.
/// `snapshot_width_points` sets the output width in points; the resulting
/// bitmap is points * backingScaleFactor pixels (aspect ratio preserved).
#[cfg(target_os = "macos")]
async fn take_native_snapshot(
    window: &WebviewWindow,
    snapshot_width_points: f64,
) -> Result<Vec<u8>, String> {
    use std::sync::Mutex as StdMutex;

    let (tx, rx) = oneshot::channel::<Result<Vec<u8>, String>>();
    let tx = std::sync::Arc::new(StdMutex::new(Some(tx)));

    let tx_for_webview = tx.clone();
    window
        .with_webview(move |webview| {
            use block2::RcBlock;
            use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
            use objc2_foundation::{NSDictionary, NSError, NSNumber};
            use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};

            // Runs on the macOS main thread (dispatched by Tauri).
            let Some(mtm) = objc2::MainThreadMarker::new() else {
                if let Some(tx) = tx_for_webview.lock().unwrap().take() {
                    let _ = tx.send(Err("with_webview closure not on main thread".into()));
                }
                return;
            };
            unsafe {
                let wk: &WKWebView = &*webview.inner().cast();
                let config = WKSnapshotConfiguration::new(mtm);
                config.setSnapshotWidth(Some(&NSNumber::new_f64(snapshot_width_points)));
                // afterScreenUpdates=true can wait forever for a hidden window
                // that never gets screen updates.
                config.setAfterScreenUpdates(false);

                let tx_block = tx_for_webview.clone();
                let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                    let result = (|| -> Result<Vec<u8>, String> {
                        if !error.is_null() {
                            return Err(format!("takeSnapshot failed: {:?}", &*error));
                        }
                        if image.is_null() {
                            return Err("takeSnapshot returned no image".into());
                        }
                        let image = &*image;
                        let tiff = image
                            .TIFFRepresentation()
                            .ok_or("snapshot has no TIFF representation")?;
                        let rep = NSBitmapImageRep::imageRepWithData(&tiff)
                            .ok_or("could not create bitmap rep from snapshot")?;
                        let png = rep
                            .representationUsingType_properties(
                                NSBitmapImageFileType::PNG,
                                &NSDictionary::new(),
                            )
                            .ok_or("could not encode snapshot as PNG")?;
                        Ok(png.to_vec())
                    })();
                    if let Some(tx) = tx_block.lock().unwrap().take() {
                        let _ = tx.send(result);
                    }
                });
                wk.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
            }
        })
        .map_err(|e| format!("with_webview failed: {e}"))?;

    tokio::time::timeout(CAPTURE_TIMEOUT, rx)
        .await
        .map_err(|_| format!("native snapshot timed out after {CAPTURE_TIMEOUT:?}"))?
        .map_err(|_| "native snapshot channel dropped".to_string())?
}

#[cfg(not(target_os = "macos"))]
async fn take_native_snapshot(
    _window: &WebviewWindow,
    _snapshot_width_points: f64,
) -> Result<Vec<u8>, String> {
    Err("native snapshot only implemented for macOS".into())
}

fn build_capture_window(
    app: &AppHandle,
    label: &str,
    width: u32,
    height: u32,
) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("capture.html".into()))
        .title("capture")
        .visible(false)
        .focused(false)
        .decorations(false)
        .resizable(false)
        .inner_size(width as f64, height as f64)
        .build()
}

fn decode_exact(
    png: Vec<u8>,
    ready: ReadySignal,
    width: u32,
    height: u32,
) -> Result<image::RgbImage, String> {
    let img = image::load_from_memory(&png).map_err(|e| format!("failed to decode PNG: {e}"))?;

    if img.width() != width || img.height() != height {
        return Err(format!(
            "captured size {}x{} != expected {}x{} (viewport was {}x{}, dpr {})",
            img.width(),
            img.height(),
            width,
            height,
            ready.inner_width,
            ready.inner_height,
            ready.device_pixel_ratio
        ));
    }

    Ok(img.to_rgb8())
}

// ---------------------------------------------------------------------------
// `--capture-spike` debug mode: all 3 widgets x 2 orientations with
// fixed fixture data (no network), written to `out_dir`.
// ---------------------------------------------------------------------------

pub async fn run_capture_spike(app: &AppHandle, out_dir: &Path) -> Result<(), String> {
    // Batch guard: also covers the gaps *between* individual captures, where
    // no window exists and no per-capture guard is held.
    let state = app.state::<CaptureState>();
    let _batch = state.hold_active();

    std::fs::create_dir_all(out_dir).map_err(|e| format!("mkdir {out_dir:?}: {e}"))?;

    for (name, html) in fixtures::all_widget_fixtures()? {
        for (suffix, orientation) in [
            ("portrait", Orientation::Portrait),
            ("landscape", Orientation::Landscape(Rotation::Cw)),
        ] {
            let jpeg = render_widget(app, html.clone(), orientation, 1200, 1600, true).await?;
            let path = out_dir.join(format!("{name}-{suffix}.jpg"));
            std::fs::write(&path, &jpeg).map_err(|e| format!("write {path:?}: {e}"))?;

            // Read-back assertion: the JPEG on disk decodes to panel size.
            let back =
                image::open(&path).map_err(|e| format!("read-back {path:?}: {e}"))?;
            if (back.width(), back.height()) != (1200, 1600) {
                return Err(format!(
                    "read-back {path:?} is {}x{}, expected 1200x1600",
                    back.width(),
                    back.height()
                ));
            }
            println!(
                "[capture-spike] wrote {} ({} bytes, 1200x1600)",
                path.display(),
                jpeg.len()
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri IPC commands used by capture.html.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct CapturePayload {
    html: String,
}

/// Called by capture.html to fetch the rendered template for this window.
#[tauri::command]
pub fn get_capture_payload(
    window: WebviewWindow,
    state: tauri::State<'_, CaptureState>,
) -> Result<CapturePayload, String> {
    let pending = state.pending.lock().unwrap();
    let entry = pending
        .get(window.label())
        .ok_or_else(|| format!("no pending capture for window {}", window.label()))?;
    Ok(CapturePayload {
        html: entry.html.clone(),
    })
}

/// Called by capture.html once the template is written and fonts are ready.
#[tauri::command]
pub fn notify_capture_ready(
    window: WebviewWindow,
    state: tauri::State<'_, CaptureState>,
    inner_width: f64,
    inner_height: f64,
    device_pixel_ratio: f64,
) -> Result<(), String> {
    deliver(
        &window,
        &state,
        Ok(ReadySignal {
            inner_width,
            inner_height,
            device_pixel_ratio,
        }),
    )
}

/// Called by capture.html if anything throws in the page.
#[tauri::command]
pub fn submit_capture_error(
    window: WebviewWindow,
    state: tauri::State<'_, CaptureState>,
    message: String,
) -> Result<(), String> {
    deliver(&window, &state, Err(message))
}

fn deliver(
    window: &WebviewWindow,
    state: &tauri::State<'_, CaptureState>,
    result: Result<ReadySignal, String>,
) -> Result<(), String> {
    let mut pending = state.pending.lock().unwrap();
    let entry = pending
        .get_mut(window.label())
        .ok_or_else(|| format!("no pending capture for window {}", window.label()))?;
    let tx = entry
        .tx
        .take()
        .ok_or("capture result already submitted")?;
    tx.send(result).map_err(|_| "capture driver gone".to_string())
}

/// Debug logging from the capture page (stdout).
#[tauri::command]
pub fn spike_log(window: WebviewWindow, message: String) {
    println!("[capture:{}] {}", window.label(), message);
}

//! Orchestrates one widget render/push end-to-end (reused unmodified by the
//! scheduler): fetch data -> `render_html` ->
//! `capture::render_widget` -> (for a push) device `upload_and_show`.
//!
//! Kept free of any Tauri-command-specific plumbing (`device.json` reads,
//! base64 data URLs for the frontend, etc. — those live in `commands.rs`) so
//! a background scheduler can call [`render_widget_config`]/
//! [`push_widget_config`] directly with its own `AppHandle`/`DeviceClient`/
//! cache dir/clock, with no dependency on the Widgets page UI.

use std::path::Path;

use chrono::{NaiveDate, NaiveDateTime};
use tauri::AppHandle;

use crate::capture::{self, Orientation, Rotation};
use crate::device::DeviceClient;
use crate::fixtures;
use crate::widgets::config::{PanelOrientation, RotateDirection, WidgetConfig, WidgetRenderConfig};
use crate::widgets::{countdown, crypto, weather};

/// Result of a successful [`push_widget_config`] call: the fresh filename
/// that was uploaded and verified as displayed, and the gallery it landed in.
#[derive(Debug, Clone, PartialEq)]
pub struct PushOutcome {
    pub filename: String,
    pub gallery: String,
}

fn to_capture_orientation(orientation: PanelOrientation, rotate: RotateDirection) -> Orientation {
    match orientation {
        PanelOrientation::Portrait => Orientation::Portrait,
        PanelOrientation::Landscape => Orientation::Landscape(match rotate {
            RotateDirection::Cw => Rotation::Cw,
            RotateDirection::Ccw => Rotation::Ccw,
        }),
    }
}

fn parse_target_date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| format!("invalid target_date {s:?} (expected YYYY-MM-DD): {e}"))
}

/// Fetches this widget's data (hitting its real API — an unreachable API
/// fails the whole call, never stale/fake data)
/// and renders it to HTML. `now` drives the crypto "updated at" stamp and
/// the countdown day count, so callers can inject a fixed clock in tests /
/// a scheduler run.
async fn render_widget_html(
    widget: &WidgetConfig,
    cache_dir: &Path,
    now: NaiveDateTime,
) -> Result<String, String> {
    let http = reqwest::Client::new();
    match widget {
        WidgetConfig::Crypto { symbols, range } => {
            let cfg = crypto::CryptoConfig {
                symbols: symbols.clone(),
                range: range.clone(),
                icon_cache_dir: cache_dir.join("crypto-icons"),
            };
            let markets = crypto::fetch(&http, &cfg).await.map_err(|e| e.to_string())?;
            let icons =
                crypto::fetch_icons(&http, crypto::DEFAULT_ICON_CDN, &cfg.icon_cache_dir, &markets)
                    .await;
            crypto::render_html(fixtures::CRYPTO_TEMPLATE, &cfg, &markets, &icons, now)
                .map_err(|e| e.to_string())
        }
        WidgetConfig::Weather {
            lat,
            lon,
            city,
            force_icon,
        } => {
            let cfg = weather::WeatherConfig {
                lat: *lat,
                lon: *lon,
                city: city.clone(),
                force_icon: force_icon.clone(),
            };
            let forecast = weather::fetch(&http, &cfg).await.map_err(|e| e.to_string())?;
            weather::render_html(fixtures::WEATHER_TEMPLATE, &cfg, &forecast)
                .map_err(|e| e.to_string())
        }
        WidgetConfig::Countdown {
            target_date,
            title,
            bg_query,
            bg_photo,
        } => {
            let target_date = parse_target_date(target_date)?;
            let cfg = countdown::CountdownConfig {
                target_date,
                title: title.clone(),
                bg_photo: bg_photo.clone(),
                bg_query: bg_query.clone(),
                cache_dir: cache_dir.join("countdown-art"),
            };
            let met = countdown::MetEndpoints::default();
            let art = countdown::fetch_art(&http, &met, &cfg, now.date())
                .await
                .map_err(|e| e.to_string())?;
            countdown::render_html(fixtures::COUNTDOWN_TEMPLATE, &cfg, &art, now.date())
                .map_err(|e| e.to_string())
        }
        // Image schedules never reach the render/capture pipeline — the
        // scheduler pushes library originals directly through
        // `commands::process_push_image`. This arm is defensive only.
        WidgetConfig::Image { .. } => {
            Err("image widgets are pushed directly, not rendered as HTML".to_string())
        }
    }
}

/// Renders `req` end-to-end to panel-ready JPEG bytes: fetch -> render_html
/// -> offscreen capture -> rotate (if landscape) -> encode (q92). Fails the
/// whole call if the widget's data API is unreachable or capture
/// fails — never returns a partially-rendered or stale image.
///
/// `rotate_to_panel` = true produces the upload-ready portrait-native bitmap
/// (used by push/scheduler). false returns the *upright* render (used by the
/// Widgets Preview) so a landscape widget previews the right way up rather
/// than as the sideways bitmap the panel physically stores.
pub async fn render_widget_config(
    app: &AppHandle,
    req: &WidgetRenderConfig,
    panel_width: u32,
    panel_height: u32,
    cache_dir: &Path,
    now: NaiveDateTime,
    rotate_to_panel: bool,
) -> Result<Vec<u8>, String> {
    let html = render_widget_html(&req.widget, cache_dir, now).await?;
    let orientation = to_capture_orientation(req.orientation, req.rotate);
    capture::render_widget(app, html, orientation, panel_width, panel_height, rotate_to_panel).await
}

/// Renders `req` and pushes it to the device: wake-if-needed
/// -> render -> `upload_and_show` under a fresh `<upload_prefix>_<timestamp>`
/// filename, verified as actually displayed. This is the
/// entry point the scheduler calls directly — it supplies its own
/// `DeviceClient`, cache dir and clock, with no dependency on the Widgets
/// page UI or `device.json`.
pub async fn push_widget_config(
    app: &AppHandle,
    client: &DeviceClient,
    req: &WidgetRenderConfig,
    panel_width: u32,
    panel_height: u32,
    cache_dir: &Path,
    gallery: &str,
    now: NaiveDateTime,
) -> Result<PushOutcome, String> {
    client.wake_if_needed().await.map_err(|e| e.to_string())?;
    // Push always uploads the panel-native (rotated) bitmap.
    let jpeg =
        render_widget_config(app, req, panel_width, panel_height, cache_dir, now, true).await?;
    let timestamp = now.format("%Y%m%d_%H%M%S").to_string();
    let result = client
        .upload_and_show(jpeg, req.widget.upload_prefix(), gallery, &timestamp)
        .await
        .map_err(|e| e.to_string())?;
    Ok(PushOutcome {
        filename: result.filename,
        gallery: result.gallery,
    })
}

// ---------------------------------------------------------------------------
// `--widgets-e2e` debug mode: automated MockDevice E2E for the acceptance
// criterion ("preview 產圖 → push → mock 端收到新檔名且 show_now"). Hits the
// *real* Open-Meteo/Binance APIs (no fixtures) and a MockDevice at
// `$WIDGETS_E2E_MOCK_URL`, mirroring exactly what `commands::preview_widget`/
// `commands::push_widget` do, minus the base64 data-URL wrapping.
// ---------------------------------------------------------------------------

/// Runs preview + push against a running MockDevice, plus one deliberately
/// bad input (unknown crypto range) to prove failures surface as `Err`
/// instead of panicking/blanking the pipeline. `mock_base_url` is a
/// MockDevice's `http://127.0.0.1:<port>`.
pub async fn run_widgets_e2e_check(app: &AppHandle, mock_base_url: &str) -> Result<(), String> {
    let cache_dir = std::env::temp_dir().join("bloomin8-widgets-e2e-cache");
    let now = chrono::Local::now().naive_local();

    // 1. Preview (no device involved): weather against the real Open-Meteo API.
    let weather_req = WidgetRenderConfig {
        widget: WidgetConfig::Weather {
            lat: 25.033,
            lon: 121.565,
            city: "Taipei".to_string(),
            force_icon: None,
        },
        orientation: PanelOrientation::Portrait,
        rotate: RotateDirection::Cw,
    };
    let jpeg = render_widget_config(app, &weather_req, 1200, 1600, &cache_dir, now, true).await?;
    let decoded = image::load_from_memory(&jpeg).map_err(|e| format!("preview JPEG decode: {e}"))?;
    if (decoded.width(), decoded.height()) != (1200, 1600) {
        return Err(format!(
            "preview size {}x{} != expected 1200x1600",
            decoded.width(),
            decoded.height()
        ));
    }
    println!("[widgets-e2e] preview OK: weather -> {} byte JPEG, 1200x1600", jpeg.len());

    // 2. Bad input must fail cleanly, not panic or silently render garbage.
    let bad_req = WidgetRenderConfig {
        widget: WidgetConfig::Crypto {
            symbols: vec!["BTC".to_string()],
            range: "not-a-range".to_string(),
        },
        orientation: PanelOrientation::Portrait,
        rotate: RotateDirection::Cw,
    };
    match render_widget_config(app, &bad_req, 1200, 1600, &cache_dir, now, true).await {
        Ok(_) => return Err("expected an invalid crypto range to fail, but it succeeded".to_string()),
        Err(e) => println!("[widgets-e2e] bad input correctly rejected: {e}"),
    }

    // 3. Push: wake_if_needed -> render -> upload_and_show against MockDevice.
    let client = DeviceClient::new(mock_base_url.to_string());
    let outcome = push_widget_config(
        app,
        &client,
        &weather_req,
        1200,
        1600,
        &cache_dir,
        "default",
        now,
    )
    .await?;
    println!(
        "[widgets-e2e] push OK: filename={} gallery={}",
        outcome.filename, outcome.gallery
    );

    // 4. Independently verify against the mock's own `/deviceInfo` (not just
    // trusting upload_and_show's internal check) that the new image is what's
    // actually "on screen".
    let info = client.info().await.map_err(|e| e.to_string())?;
    let shown = info.image.unwrap_or_default();
    if !shown.ends_with(&outcome.filename) {
        return Err(format!(
            "post-push deviceInfo.image {shown:?} does not end with pushed filename {:?}",
            outcome.filename
        ));
    }
    println!("[widgets-e2e] verified via GET /deviceInfo: image={shown}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_target_date_accepts_iso_and_rejects_garbage() {
        assert!(parse_target_date("2026-12-25").is_ok());
        assert!(parse_target_date("12/25/2026").is_err());
        assert!(parse_target_date("not-a-date").is_err());
    }

    #[test]
    fn to_capture_orientation_maps_portrait_and_landscape_rotation() {
        assert_eq!(
            to_capture_orientation(PanelOrientation::Portrait, RotateDirection::Cw),
            Orientation::Portrait
        );
        assert_eq!(
            to_capture_orientation(PanelOrientation::Landscape, RotateDirection::Ccw),
            Orientation::Landscape(Rotation::Ccw)
        );
        assert_eq!(
            to_capture_orientation(PanelOrientation::Landscape, RotateDirection::Cw),
            Orientation::Landscape(Rotation::Cw)
        );
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("bloomin8-render-service-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn render_widget_html_renders_countdown_from_a_local_photo_with_no_network() {
        let dir = tmp_dir("countdown-photo");
        let photo = dir.join("me.jpg");
        std::fs::write(&photo, [0xFFu8, 0xD8, 0xFF, 0xD9]).unwrap();

        let widget = WidgetConfig::Countdown {
            target_date: "2026-12-25".to_string(),
            title: "Christmas".to_string(),
            bg_query: "unused".to_string(),
            bg_photo: Some(photo),
        };
        let now = NaiveDate::from_ymd_opt(2026, 7, 7)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();

        let html = render_widget_html(&widget, &dir, now).await.unwrap();
        assert!(!html.contains("{{"));
        assert!(html.contains("Christmas"));
    }

    #[tokio::test]
    async fn render_widget_html_rejects_image_variant_without_touching_network() {
        // The scheduler pushes image schedules directly (never through the
        // render pipeline); this arm is defensive and must fail cleanly.
        let dir = tmp_dir("image-defensive");
        let widget = WidgetConfig::Image {
            library_id: "lib-1".to_string(),
            mode: "auto".to_string(),
            border: "black".to_string(),
        };
        let now = NaiveDate::from_ymd_opt(2026, 7, 7)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();

        let err = render_widget_html(&widget, &dir, now).await.unwrap_err();
        assert!(err.contains("pushed directly"));
    }

    #[tokio::test]
    async fn render_widget_html_fails_on_invalid_target_date_without_touching_network() {
        let dir = tmp_dir("countdown-bad-date");
        let widget = WidgetConfig::Countdown {
            target_date: "not-a-date".to_string(),
            title: "X".to_string(),
            bg_query: "unused".to_string(),
            bg_photo: Some(PathBuf::from("/nonexistent-file-for-test")),
        };
        let now = NaiveDate::from_ymd_opt(2026, 7, 7)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();

        let err = render_widget_html(&widget, &dir, now).await.unwrap_err();
        assert!(err.contains("invalid target_date"));
    }
}

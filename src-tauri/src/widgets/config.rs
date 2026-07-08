//! Widget configuration model shared by the Widgets page UI and the
//! scheduler: a serializable description of "which widget, with
//! what user-facing parameters, in what orientation" that both a Tauri
//! command and a headless cron job can construct and hand to
//! `render_service::{render_widget_config, push_widget_config}`.
//!
//! `target_date` is a plain `YYYY-MM-DD` string rather than `chrono::NaiveDate`
//! so this type needs no extra `chrono` Cargo feature (`serde` support for
//! chrono types is feature-gated) — `render_service` parses it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Per-widget user-facing parameters. Tagged so the frontend/scheduler can
/// send/store this as `{"kind": "crypto", ...}` JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WidgetConfig {
    Crypto {
        /// e.g. `["BTC", "ETH"]` — bare symbols get `USDT` appended (see
        /// `crypto::normalize_symbol`).
        symbols: Vec<String>,
        /// One of `"24h"`, `"7d"`, `"30d"`.
        range: String,
    },
    Weather {
        lat: f64,
        lon: f64,
        city: String,
        /// Debug/preview-only condition override (`--force-icon` equivalent).
        #[serde(default)]
        force_icon: Option<String>,
    },
    Countdown {
        /// `YYYY-MM-DD`, parsed by `render_service`.
        target_date: String,
        title: String,
        bg_query: String,
        /// Local photo path; when set, skips the Met Museum artwork fetch
        /// entirely (no network).
        #[serde(default)]
        bg_photo: Option<PathBuf>,
    },
    /// A fixed image from the local library (`library.rs`), pushed with the
    /// same display settings the manual push dialog offers. Unlike the other
    /// variants there's no data fetch/HTML render/capture — the scheduler reads
    /// the library original and runs it through `commands::process_push_image`
    /// directly. Orientation comes from the enclosing
    /// [`WidgetRenderConfig::orientation`] (a `flatten`ed duplicate here would
    /// collide on the JSON `orientation` key); rotation is always clockwise,
    /// matching the push dialog's default.
    Image {
        /// A `library::LibraryItem::id`.
        library_id: String,
        /// `"auto"` | `"fit"` | `"fill"`.
        mode: String,
        /// `"white"` | `"black"`.
        border: String,
    },
}

impl WidgetConfig {
    /// Stable filename prefix (`<prefix>_<timestamp>.jpg`) so each widget
    /// kind keeps its own upload lineage on the device and repeated pushes
    /// of *different* widgets never collide on a filename.
    pub fn upload_prefix(&self) -> &'static str {
        match self {
            WidgetConfig::Crypto { .. } => "widget-crypto",
            WidgetConfig::Weather { .. } => "widget-weather",
            WidgetConfig::Countdown { .. } => "widget-countdown",
            // Matches the manual push dialog's `photo_<ts>.jpg` lineage.
            WidgetConfig::Image { .. } => "photo",
        }
    }
}

/// Panel orientation as chosen by the user — distinct from
/// `capture::Orientation`, which additionally bakes in the rotation
/// direction. Kept separate so this type stays a plain, serde-friendly
/// UI-facing model with no dependency on the capture pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelOrientation {
    Portrait,
    Landscape,
}

/// Which way a landscape capture is rotated back to portrait-native before
/// upload; irrelevant when `orientation` is `Portrait`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotateDirection {
    Cw,
    Ccw,
}

impl Default for RotateDirection {
    fn default() -> Self {
        RotateDirection::Cw
    }
}

/// Everything needed to render one widget instance: its data/parameters plus
/// how it should be captured. Both the Widgets page's Tauri commands
/// (`preview_widget`/`push_widget`) and the scheduler construct this and
/// pass it to `render_service`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetRenderConfig {
    #[serde(flatten)]
    pub widget: WidgetConfig,
    pub orientation: PanelOrientation,
    #[serde(default)]
    pub rotate: RotateDirection,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crypto_config_serializes_with_kind_tag() {
        let cfg = WidgetConfig::Crypto {
            symbols: vec!["BTC".into()],
            range: "24h".into(),
        };
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["kind"], "crypto");
        assert_eq!(json["range"], "24h");
    }

    #[test]
    fn render_config_flattens_widget_fields_alongside_orientation() {
        let req = WidgetRenderConfig {
            widget: WidgetConfig::Weather {
                lat: 25.0,
                lon: 121.0,
                city: "Taipei".into(),
                force_icon: None,
            },
            orientation: PanelOrientation::Landscape,
            rotate: RotateDirection::Ccw,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["kind"], "weather");
        assert_eq!(json["orientation"], "landscape");
        assert_eq!(json["rotate"], "ccw");
    }

    #[test]
    fn upload_prefix_is_distinct_per_widget_kind() {
        let crypto = WidgetConfig::Crypto {
            symbols: vec![],
            range: "24h".into(),
        };
        let weather = WidgetConfig::Weather {
            lat: 0.0,
            lon: 0.0,
            city: "".into(),
            force_icon: None,
        };
        let countdown = WidgetConfig::Countdown {
            target_date: "2026-01-01".into(),
            title: "".into(),
            bg_query: "".into(),
            bg_photo: None,
        };
        assert_ne!(crypto.upload_prefix(), weather.upload_prefix());
        assert_ne!(weather.upload_prefix(), countdown.upload_prefix());
        assert_ne!(crypto.upload_prefix(), countdown.upload_prefix());
    }

    #[test]
    fn image_config_round_trips_with_kind_tag_and_photo_prefix() {
        let cfg = WidgetConfig::Image {
            library_id: "abc123".into(),
            mode: "fit".into(),
            border: "white".into(),
        };
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["kind"], "image");
        assert_eq!(json["library_id"], "abc123");
        assert_eq!(json["mode"], "fit");
        assert_eq!(json["border"], "white");

        let back: WidgetConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back, cfg);
        assert_eq!(cfg.upload_prefix(), "photo");
    }

    #[test]
    fn image_render_config_round_trips_with_wrapper_orientation() {
        // Regression guard: the Image variant must NOT carry its own
        // `orientation` — the flattened `WidgetRenderConfig.orientation` owns
        // that key, and a duplicate would make deserialization fail.
        let req = WidgetRenderConfig {
            widget: WidgetConfig::Image {
                library_id: "lib-1".into(),
                mode: "fill".into(),
                border: "black".into(),
            },
            orientation: PanelOrientation::Landscape,
            rotate: RotateDirection::Cw,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["kind"], "image");
        assert_eq!(json["orientation"], "landscape");
        let back: WidgetRenderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn rotate_defaults_to_cw_when_absent_from_json() {
        let json = serde_json::json!({
            "kind": "weather",
            "lat": 25.0,
            "lon": 121.0,
            "city": "Taipei",
            "orientation": "landscape",
        });
        let req: WidgetRenderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(req.rotate, RotateDirection::Cw);
    }
}

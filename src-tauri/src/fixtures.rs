//! Deterministic widget fixtures for `--capture-spike` / golden tests.
//!
//! Every fixture feeds a widget's *pure* `render_html` with a hand-built
//! struct — no network, no wall clock — so the resulting HTML (and therefore
//! the captured golden image) is bit-stable across runs. The countdown
//! background is a small procedurally-generated gradient JPEG (base64 data
//! URI), so no binary fixture asset needs to live in the repo besides the
//! golden images themselves.

use std::path::PathBuf;

use chrono::NaiveDate;

use crate::widgets::{countdown, crypto, weather};

pub const WEATHER_TEMPLATE: &str = include_str!("../templates/weather-template.html");
pub const CRYPTO_TEMPLATE: &str = include_str!("../templates/crypto-template.html");
pub const COUNTDOWN_TEMPLATE: &str = include_str!("../templates/countdown-template.html");

/// All three widgets' fixture HTML, keyed by the file-name stem used for
/// spike output and golden images.
pub fn all_widget_fixtures() -> Result<Vec<(&'static str, String)>, String> {
    Ok(vec![
        ("weather", weather_fixture_html().map_err(|e| e.to_string())?),
        ("crypto", crypto_fixture_html().map_err(|e| e.to_string())?),
        (
            "countdown",
            countdown_fixture_html().map_err(|e| e.to_string())?,
        ),
    ])
}

// ---------------------------------------------------------------------------
// Weather: rainy Taipei afternoon (theme `rain`, 5-slot forecast strip).
// ---------------------------------------------------------------------------

pub fn weather_fixture_html() -> Result<String, crate::widgets::WidgetError> {
    let config = weather::WeatherConfig {
        lat: 25.033,
        lon: 121.565,
        city: "TAIPEI".to_string(),
        force_icon: None,
    };

    // Two days of hourly data (2026-07-06 .. 07-07); "now" is 15:12 on day 1,
    // so the strip covers 15:00, 18:00, 21:00, 00:00, 03:00.
    let mut time = Vec::new();
    for day in [6u32, 7u32] {
        for h in 0..24u32 {
            time.push(format!("2026-07-{day:02}T{h:02}:00"));
        }
    }
    let mut temperature_2m: Vec<f64> = (0..48).map(|i| 21.0 + ((i % 12) as f64) * 0.3).collect();
    let mut precipitation_probability: Vec<Option<f64>> =
        (0..48).map(|i| Some(((i * 7) % 90) as f64)).collect();
    let mut weather_code: Vec<i64> = vec![3; 48];

    // The five displayed slots (indices 15, 18, 21, 24, 27): rain, rain,
    // drizzle, partly cloudy, clear — exercises several forecast icons.
    for (idx, temp, pop, code) in [
        (15usize, 24.0, 68.0, 61i64),
        (18, 23.0, 55.0, 61),
        (21, 22.0, 40.0, 51),
        (24, 21.0, 20.0, 2),
        (27, 20.0, 5.0, 0),
    ] {
        temperature_2m[idx] = temp;
        precipitation_probability[idx] = Some(pop);
        weather_code[idx] = code;
    }

    let forecast = weather::Forecast {
        current: weather::CurrentData {
            time: "2026-07-06T15:12".to_string(),
            temperature_2m: 24.3,
            apparent_temperature: 26.1,
            relative_humidity_2m: 82.0,
            weather_code: 61, // Light Rain -> theme `rain`
        },
        hourly: weather::HourlyData {
            time,
            temperature_2m,
            precipitation_probability,
            weather_code,
        },
    };

    weather::render_html(WEATHER_TEMPLATE, &config, &forecast)
}

// ---------------------------------------------------------------------------
// Crypto: BTC/ETH/SOL, 24h range, single-line stack, lettered icon fallback.
// ---------------------------------------------------------------------------

/// Deterministic pseudo-kline series: `base` with a drift and a sine wobble.
fn closes(base: f64, drift_per_step: f64, amplitude: f64) -> Vec<f64> {
    (0..24)
        .map(|i| base + drift_per_step * i as f64 + amplitude * ((i as f64) * 0.7).sin())
        .collect()
}

pub fn crypto_fixture_html() -> Result<String, crate::widgets::WidgetError> {
    let config = crypto::CryptoConfig {
        symbols: vec!["BTC".into(), "ETH".into(), "SOL".into()],
        range: "24h".to_string(),
        icon_cache_dir: PathBuf::new(), // unused by the pure renderer
    };

    let markets = vec![
        crypto::SymbolMarket {
            symbol: "BTCUSDT".to_string(),
            price: 67234.5,
            change_percent: 2.34,
            closes: closes(65800.0, 60.0, 320.0),
            range_open: 65650.0,
        },
        crypto::SymbolMarket {
            symbol: "ETHUSDT".to_string(),
            price: 3245.8,
            change_percent: -1.12,
            closes: closes(3300.0, -2.5, 28.0),
            range_open: 3310.0,
        },
        crypto::SymbolMarket {
            symbol: "SOLUSDT".to_string(),
            price: 152.4,
            change_percent: 0.45,
            closes: closes(150.0, 0.1, 3.2),
            range_open: 151.7,
        },
    ];

    // Deterministic lettered-circle icons (no CDN fetch, no cache reads).
    let icons: Vec<String> = ["BTC", "ETH", "SOL"]
        .iter()
        .map(|b| crypto::icon_fallback_html(b))
        .collect();

    let now = NaiveDate::from_ymd_opt(2026, 7, 6)
        .unwrap()
        .and_hms_opt(15, 4, 0)
        .unwrap();

    crypto::render_html(CRYPTO_TEMPLATE, &config, &markets, &icons, now)
}

// ---------------------------------------------------------------------------
// Countdown: Christmas 2026, procedurally generated gradient background.
// ---------------------------------------------------------------------------

/// A small (480x640) dusk-gradient JPEG with a pale sun disc — asymmetric on
/// purpose so mis-rotation would be visually obvious in the golden image.
pub fn fixture_background_jpeg() -> Vec<u8> {
    let (w, h) = (480u32, 640u32);
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        // Vertical dusk gradient: deep indigo (top) -> warm amber (bottom).
        let r = 30.0 + 190.0 * fy + 10.0 * fx;
        let g = 30.0 + 90.0 * fy;
        let b = 90.0 + 20.0 * (1.0 - fy);
        // Soft "sun" disc in the upper-left third.
        let (cx, cy, rad) = (0.32f32, 0.28f32, 0.16f32);
        let d = ((fx - cx).powi(2) + (fy - cy) .powi(2)).sqrt();
        let glow = (1.0 - (d / rad).min(1.0)).powi(2);
        let px = |v: f32| v.clamp(0.0, 255.0) as u8;
        image::Rgb([
            px(r + 160.0 * glow),
            px(g + 150.0 * glow),
            px(b + 120.0 * glow),
        ])
    });
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode_image(&img)
        .expect("in-memory JPEG encode cannot fail");
    jpeg
}

pub fn countdown_fixture_html() -> Result<String, crate::widgets::WidgetError> {
    use base64::Engine as _;

    let config = countdown::CountdownConfig {
        target_date: NaiveDate::from_ymd_opt(2026, 12, 25).unwrap(),
        title: "Christmas".to_string(),
        bg_photo: None,
        bg_query: "cat".to_string(),
        cache_dir: PathBuf::new(), // unused by the pure renderer
    };
    let art = countdown::CountdownArt {
        bg_uri: format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(fixture_background_jpeg())
        ),
        credit: "Procedural gradient — golden-test fixture".to_string(),
    };
    let today = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();

    countdown::render_html(COUNTDOWN_TEMPLATE, &config, &art, today)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_fixture_renders_rain_theme_with_full_strip() {
        let html = weather_fixture_html().unwrap();
        assert!(!html.contains("{{"), "unreplaced placeholders remain");
        assert!(html.contains("theme-rain"));
        assert!(html.contains("TAIPEI"));
        assert!(html.contains(">Now<"));
        assert_eq!(html.matches("class=\"cell\"").count(), 5);
    }

    #[test]
    fn crypto_fixture_renders_three_cards_with_sparklines() {
        let html = crypto_fixture_html().unwrap();
        // Note: the crypto template legitimately contains "{{COLS_*}}" inside
        // a documentation comment, so check the real placeholders explicitly.
        for ph in [
            "{{CARDS}}",
            "{{COLS_PORTRAIT}}",
            "{{COLS_LANDSCAPE}}",
            "{{RANGE_LABEL}}",
            "{{UPDATED_AT}}",
        ] {
            assert!(!html.contains(ph), "unreplaced placeholder: {ph}");
        }
        assert!(html.contains("BTC"));
        assert!(html.contains("ETH"));
        assert!(html.contains("SOL"));
        // One dashed range-open line + endpoint dot per card.
        assert_eq!(html.matches("stroke-dasharray=\"8 8\"").count(), 3);
        assert_eq!(html.matches("class=\"dot\"").count(), 3);
    }

    #[test]
    fn countdown_fixture_renders_days_until_christmas() {
        let html = countdown_fixture_html().unwrap();
        assert!(!html.contains("{{"), "unreplaced placeholders remain");
        assert!(html.contains("172")); // 2026-07-06 -> 2026-12-25
        assert!(html.contains("days until"));
        assert!(html.contains("Christmas"));
        assert!(html.contains("data:image/jpeg;base64,"));
    }

    #[test]
    fn fixture_background_is_deterministic_valid_jpeg() {
        let a = fixture_background_jpeg();
        let b = fixture_background_jpeg();
        assert_eq!(a, b, "background generation must be deterministic");
        let img = image::load_from_memory(&a).unwrap();
        assert_eq!((img.width(), img.height()), (480, 640));
    }
}

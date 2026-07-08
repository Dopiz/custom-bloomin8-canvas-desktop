//! Today's-weather widget.
//!
//! Ports `~/.claude/skills/bloomin8-canvas/scripts/weather.py` 1:1:
//! Open-Meteo `current` + `hourly` forecast (`timezone=auto`), a 5-slot x 2h
//! outlook strip anchored at the current hour (first slot labelled "Now"),
//! a background gradient theme keyed off the weather *condition group*
//! (not the clock), current-hour precipitation probability sourced from the
//! hourly series (absent from `current`), and a debug `force_icon` override.

use reqwest::Client;
use serde::Deserialize;

use super::template;
use super::WidgetError;

pub const DEFAULT_API_URL: &str = "https://api.open-meteo.com/v1/forecast";

/// WMO weather code -> (display label, icon/condition group). Mirrors the
/// `WMO` dict in `weather.py` verbatim.
fn wmo_lookup(code: i64) -> (&'static str, &'static str) {
    match code {
        0 => ("Clear Sky", "clear"),
        1 => ("Mainly Clear", "clear"),
        2 => ("Partly Cloudy", "partly"),
        3 => ("Overcast", "overcast"),
        45 => ("Fog", "fog"),
        48 => ("Rime Fog", "fog"),
        51 => ("Light Drizzle", "drizzle"),
        53 => ("Drizzle", "drizzle"),
        55 => ("Heavy Drizzle", "drizzle"),
        56 => ("Freezing Drizzle", "drizzle"),
        57 => ("Freezing Drizzle", "drizzle"),
        61 => ("Light Rain", "rain"),
        63 => ("Rain", "rain"),
        65 => ("Heavy Rain", "rain"),
        66 => ("Freezing Rain", "rain"),
        67 => ("Freezing Rain", "rain"),
        71 => ("Light Snow", "snow"),
        73 => ("Snow", "snow"),
        75 => ("Heavy Snow", "snow"),
        77 => ("Snow Grains", "snow"),
        80 => ("Light Showers", "rain"),
        81 => ("Rain Showers", "rain"),
        82 => ("Heavy Showers", "rain"),
        85 => ("Snow Showers", "snow"),
        86 => ("Snow Showers", "snow"),
        95 => ("Thunderstorm", "thunder"),
        96 => ("Thunderstorm", "thunder"),
        99 => ("Thunderstorm", "thunder"),
        _ => ("Unknown", "overcast"),
    }
}

/// Icon/condition group -> background gradient theme class (`ICON_THEME` in
/// `weather.py`). The theme follows the condition, never the clock.
fn pick_theme(icon: &str) -> &'static str {
    match icon {
        "clear" => "clear",
        "partly" => "cloudy",
        "overcast" => "cloudy",
        "drizzle" => "rain",
        "rain" => "rain",
        "snow" => "snow",
        "thunder" => "thunder",
        "fog" => "fog",
        _ => "cloudy",
    }
}

/// All valid `force_icon` values (used for validation + tests).
pub const ICON_GROUPS: &[&str] = &[
    "clear", "partly", "overcast", "fog", "drizzle", "rain", "snow", "thunder",
];

#[derive(Debug, Clone)]
pub struct WeatherConfig {
    pub lat: f64,
    pub lon: f64,
    pub city: String,
    /// Debug/preview-only override of the condition icon+theme (mirrors
    /// `--force-icon`); `None` in normal operation.
    pub force_icon: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CurrentData {
    pub time: String,
    pub temperature_2m: f64,
    pub apparent_temperature: f64,
    pub relative_humidity_2m: f64,
    pub weather_code: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HourlyData {
    pub time: Vec<String>,
    pub temperature_2m: Vec<f64>,
    pub precipitation_probability: Vec<Option<f64>>,
    pub weather_code: Vec<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Forecast {
    pub current: CurrentData,
    pub hourly: HourlyData,
}

/// Fetch the forecast against the real Open-Meteo API.
pub async fn fetch(http: &Client, config: &WeatherConfig) -> Result<Forecast, WidgetError> {
    fetch_from(http, DEFAULT_API_URL, config).await
}

/// Same as [`fetch`] but with an injectable API base URL, so tests can point
/// at a mock server.
pub async fn fetch_from(
    http: &Client,
    api_url: &str,
    config: &WeatherConfig,
) -> Result<Forecast, WidgetError> {
    let resp = http
        .get(api_url)
        .query(&[
            ("latitude", config.lat.to_string()),
            ("longitude", config.lon.to_string()),
            (
                "current",
                "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code".to_string(),
            ),
            (
                "hourly",
                "temperature_2m,precipitation_probability,weather_code".to_string(),
            ),
            ("timezone", "auto".to_string()),
            ("forecast_days", "2".to_string()),
        ])
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()?;
    let forecast: Forecast = resp.json().await?;
    Ok(forecast)
}

fn hour_label(iso_time: &str) -> Result<String, WidgetError> {
    let dt = parse_naive(iso_time)?;
    use chrono::Timelike;
    let h = dt.hour();
    let suffix = if h < 12 { "AM" } else { "PM" };
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    Ok(format!("{h12} {suffix}"))
}

fn parse_naive(iso_time: &str) -> Result<chrono::NaiveDateTime, WidgetError> {
    chrono::NaiveDateTime::parse_from_str(iso_time, "%Y-%m-%dT%H:%M")
        .map_err(|e| WidgetError::Data(format!("bad Open-Meteo timestamp {iso_time:?}: {e}")))
}

/// Five slots anchored at the current hour, 2 h apart: now, +2h, +4h, +6h, +8h.
const FORECAST_STEP_HOURS: usize = 2;

fn build_forecast_cells(hourly: &HourlyData, now_idx: usize) -> Result<String, WidgetError> {
    let mut cells = Vec::with_capacity(5);
    for step in 0..5usize {
        let i = now_idx + step * FORECAST_STEP_HOURS;
        if i >= hourly.time.len() {
            return Err(WidgetError::Data(format!(
                "hourly forecast too short for slot +{}h",
                step * FORECAST_STEP_HOURS
            )));
        }
        let (_, icon) = wmo_lookup(hourly.weather_code[i]);
        let pop_txt = match hourly.precipitation_probability[i] {
            Some(p) => format!("{}%", p.round() as i64),
            None => "\u{2013}".to_string(),
        };
        let label = if step == 0 {
            "Now".to_string()
        } else {
            hour_label(&hourly.time[i])?
        };
        cells.push(format!(
            "<div class=\"cell\">\
<div class=\"cell-time\">{label}</div>\
<svg class=\"cell-icon\" viewBox=\"0 0 100 100\"><use href=\"#i-{icon}\"/></svg>\
<div class=\"cell-temp\">{}&deg;</div>\
<div class=\"cell-pop\">{pop_txt}</div>\
</div>",
            hourly.temperature_2m[i].round() as i64,
        ));
    }
    Ok(cells.join("\n"))
}

/// Pure render: template + config + fetched forecast -> HTML. All "now"
/// context comes from `forecast.current.time` (the API's own local clock),
/// not the wall clock, so this is fully deterministic given its inputs.
pub fn render_html(
    template_html: &str,
    config: &WeatherConfig,
    forecast: &Forecast,
) -> Result<String, WidgetError> {
    let cur = &forecast.current;
    let hourly = &forecast.hourly;

    // `current` has no precipitation probability; find it in `hourly` by
    // matching the "YYYY-MM-DDTHH" hour prefix (times are local thanks to
    // timezone=auto).
    let hour_key = cur
        .time
        .get(0..13)
        .ok_or_else(|| WidgetError::Data(format!("malformed current.time: {:?}", cur.time)))?;
    let now_idx = hourly
        .time
        .iter()
        .position(|t| t.get(0..13) == Some(hour_key))
        .ok_or_else(|| {
            WidgetError::Data(format!(
                "current time {} not found in hourly forecast",
                cur.time
            ))
        })?;
    let pop_now = match hourly.precipitation_probability.get(now_idx).copied().flatten() {
        Some(p) => p.round() as i64,
        None => 0,
    };

    let (condition_default, mut icon) = wmo_lookup(cur.weather_code);
    let mut condition_forced: Option<String> = None;
    if let Some(forced) = &config.force_icon {
        icon = ICON_GROUPS
            .iter()
            .find(|g| *g == forced)
            .copied()
            .ok_or_else(|| WidgetError::Data(format!("unknown force_icon group: {forced}")))?;
        condition_forced = Some(format!("{}{} (forced)", icon[..1].to_uppercase(), &icon[1..]));
    }
    let condition: &str = condition_forced.as_deref().unwrap_or(condition_default);

    let now_local = parse_naive(&cur.time)?;
    let cells = build_forecast_cells(hourly, now_idx)?;

    let date_s = now_local.format("%A, %B %-d").to_string();
    let time_s = now_local.format("%-I:%M%p").to_string().to_lowercase();
    let theme = pick_theme(icon);
    let temp_s = (cur.temperature_2m.round() as i64).to_string();
    let feels_s = (cur.apparent_temperature.round() as i64).to_string();
    let humidity_s = (cur.relative_humidity_2m.round() as i64).to_string();
    let precip_s = pop_now.to_string();

    Ok(template::render(
        template_html,
        &[
            ("CITY", config.city.as_str()),
            ("DATE", date_s.as_str()),
            ("TIME", time_s.as_str()),
            ("THEME", theme),
            ("TEMP", temp_s.as_str()),
            ("CONDITION", condition),
            ("ICON", icon),
            ("PRECIP", precip_s.as_str()),
            ("FEELS", feels_s.as_str()),
            ("HUMIDITY", humidity_s.as_str()),
            ("CELLS", cells.as_str()),
        ],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Json, Router};
    use serde_json::json;

    const TEMPLATE: &str = include_str!("../../templates/weather-template.html");

    /// `current_time` must be one of the generated hourly timestamps, e.g.
    /// `"2026-07-07T09:00"`. 24 hourly slots (one per hour of the day) give
    /// every test plenty of room for the +12h forecast slot.
    fn sample_forecast(current_time: &str, code: i64) -> Forecast {
        let times: Vec<String> = (0..24).map(|h| format!("2026-07-07T{h:02}:00")).collect();
        Forecast {
            current: CurrentData {
                time: current_time.to_string(),
                temperature_2m: 28.4,
                apparent_temperature: 31.2,
                relative_humidity_2m: 70.0,
                weather_code: code,
            },
            hourly: HourlyData {
                time: times,
                temperature_2m: (0..24).map(|i| 20.0 + i as f64).collect(),
                precipitation_probability: (0..24).map(|i| Some((i * 5) as f64)).collect(),
                weather_code: (0..24).map(|_| code).collect(),
            },
        }
    }

    fn cfg() -> WeatherConfig {
        WeatherConfig {
            lat: 25.033,
            lon: 121.565,
            city: "Taipei".to_string(),
            force_icon: None,
        }
    }

    #[test]
    fn condition_group_maps_to_expected_theme() {
        assert_eq!(pick_theme("clear"), "clear");
        assert_eq!(pick_theme("partly"), "cloudy");
        assert_eq!(pick_theme("overcast"), "cloudy");
        assert_eq!(pick_theme("fog"), "fog");
        assert_eq!(pick_theme("drizzle"), "rain");
        assert_eq!(pick_theme("rain"), "rain");
        assert_eq!(pick_theme("snow"), "snow");
        assert_eq!(pick_theme("thunder"), "thunder");
    }

    #[test]
    fn render_html_has_no_leftover_placeholders_for_each_condition_group() {
        for code in [0, 2, 3, 45, 61, 71, 95] {
            let forecast = sample_forecast("2026-07-07T09:00", code);
            let html = render_html(TEMPLATE, &cfg(), &forecast).unwrap();
            assert!(!html.contains("{{"), "code {code} left placeholders");
        }
    }

    #[test]
    fn render_html_labels_first_slot_now() {
        let forecast = sample_forecast("2026-07-07T09:00", 0);
        let html = render_html(TEMPLATE, &cfg(), &forecast).unwrap();
        assert!(html.contains("cell-time\">Now<"));
    }

    #[test]
    fn force_icon_overrides_condition_and_theme() {
        let forecast = sample_forecast("2026-07-07T09:00", 0); // clear
        let mut config = cfg();
        config.force_icon = Some("thunder".to_string());
        let html = render_html(TEMPLATE, &config, &forecast).unwrap();
        assert!(html.contains("theme-thunder"));
        assert!(html.contains("Thunder (forced)"));
    }

    #[tokio::test]
    async fn fetch_from_parses_current_and_hourly() {
        let app = Router::new().route(
            "/forecast",
            get(|| async {
                Json(json!({
                    "current": {
                        "time": "2026-07-07T09:00",
                        "temperature_2m": 28.4,
                        "apparent_temperature": 31.2,
                        "relative_humidity_2m": 70,
                        "weather_code": 0,
                    },
                    "hourly": {
                        "time": (0..24).map(|h| format!("2026-07-07T{h:02}:00")).collect::<Vec<_>>(),
                        "temperature_2m": (0..24).map(|i| 20.0 + i as f64).collect::<Vec<f64>>(),
                        "precipitation_probability": (0..24).map(|i| (i * 5) as i64).collect::<Vec<i64>>(),
                        "weather_code": (0..24).map(|_| 0).collect::<Vec<i64>>(),
                    }
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });

        let http = Client::new();
        let forecast = fetch_from(&http, &format!("http://{addr}/forecast"), &cfg())
            .await
            .unwrap();
        assert_eq!(forecast.current.weather_code, 0);
        assert_eq!(forecast.hourly.time.len(), 24);
    }

    #[tokio::test]
    async fn fetch_from_fails_when_unreachable() {
        let http = Client::new();
        let err = fetch_from(&http, "http://127.0.0.1:1/forecast", &cfg())
            .await
            .unwrap_err();
        assert!(matches!(err, WidgetError::Http(_)));
    }
}

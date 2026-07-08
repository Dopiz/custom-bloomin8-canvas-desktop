//! Countdown / days-since widget.
//!
//! Ports `~/.claude/skills/bloomin8-canvas/scripts/countdown.py` 1:1:
//! calendar-day difference (with "today" injected, not read from the wall
//! clock), past dates flip the label to "days since", background art comes
//! from the Met Museum's public API picked deterministically by
//! `abs(days) % results` (stable within a day, rotates daily), fetched art
//! is cached (image bytes + credit line) to a caller-supplied directory so
//! offline/cron runs fall back to the cache, and a user-supplied photo path
//! skips the network entirely. The chosen artwork is embedded into the HTML
//! as a base64 data URI (never an external `url()` reference).

use std::path::{Path, PathBuf};

use base64::Engine;
use chrono::NaiveDate;
use reqwest::Client;
use serde::Deserialize;

use super::template;
use super::WidgetError;

pub const DEFAULT_MET_SEARCH_URL: &str =
    "https://collectionapi.metmuseum.org/public/collection/v1/search";
/// `{}` is replaced with the object id.
pub const DEFAULT_MET_OBJECT_URL_TEMPLATE: &str =
    "https://collectionapi.metmuseum.org/public/collection/v1/objects/{}";

/// Met Museum API endpoints; overridable so tests can point at a mock server.
#[derive(Debug, Clone)]
pub struct MetEndpoints {
    pub search_url: String,
    /// Must contain a single `{}` placeholder for the object id.
    pub object_url_template: String,
}

impl Default for MetEndpoints {
    fn default() -> Self {
        Self {
            search_url: DEFAULT_MET_SEARCH_URL.to_string(),
            object_url_template: DEFAULT_MET_OBJECT_URL_TEMPLATE.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CountdownConfig {
    pub target_date: NaiveDate,
    pub title: String,
    /// Local photo path; when set, skips artwork fetch/cache entirely.
    pub bg_photo: Option<PathBuf>,
    pub bg_query: String,
    /// Directory used to cache fetched artwork (`met_<id>.jpg` + `.txt`
    /// credit sidecar) and as the offline fallback source.
    pub cache_dir: PathBuf,
}

/// Resolved background art: an already-embeddable data URI plus its credit
/// line (empty for user photos or cache-fallback-with-no-metadata cases).
#[derive(Debug, Clone, PartialEq)]
pub struct CountdownArt {
    pub bg_uri: String,
    pub credit: String,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(rename = "objectIDs")]
    object_ids: Option<Vec<i64>>,
}

#[derive(Debug, Deserialize)]
struct ObjectResponse {
    #[serde(rename = "isPublicDomain")]
    is_public_domain: Option<bool>,
    #[serde(rename = "primaryImageSmall")]
    primary_image_small: Option<String>,
    title: Option<String>,
    #[serde(rename = "artistDisplayName")]
    artist_display_name: Option<String>,
}

fn data_uri(mime: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// Calendar-day difference: positive when `target` is in the future relative
/// to `today`, negative when in the past. Mirrors `(target - date.today()).days`.
pub fn days_until(target: NaiveDate, today: NaiveDate) -> i64 {
    (target - today).num_days()
}

pub fn countdown_label(days: i64) -> &'static str {
    if days >= 0 {
        "days until"
    } else {
        "days since"
    }
}

/// Fetch (and cache) background artwork for `query`, deterministically
/// picked by `pick` (mirrors `fetch_background` in `countdown.py`).
///
/// On any API failure, falls back to the newest-sorted cached `.jpg` in
/// `cache_dir`, matching the Python reference's offline-cron guarantee. If
/// the cache is also empty, returns `Err` (failure policy: never
/// stale/fabricated data — but a genuinely cached image is the documented
/// exception, not a violation of it).
pub async fn fetch_background(
    http: &Client,
    met: &MetEndpoints,
    query: &str,
    pick: usize,
    cache_dir: &Path,
) -> Result<(Vec<u8>, String), WidgetError> {
    std::fs::create_dir_all(cache_dir)?;

    match fetch_background_live(http, met, query, pick, cache_dir).await {
        Ok(result) => Ok(result),
        Err(_live_err) => {
            let mut cached: Vec<PathBuf> = std::fs::read_dir(cache_dir)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jpg"))
                .collect();
            cached.sort();
            if cached.is_empty() {
                return Err(WidgetError::Data(format!(
                    "artwork API failed and no cached backgrounds available in {}",
                    cache_dir.display()
                )));
            }
            let path = &cached[pick % cached.len()];
            let bytes = std::fs::read(path)?;
            Ok((bytes, String::new()))
        }
    }
}

async fn fetch_background_live(
    http: &Client,
    met: &MetEndpoints,
    query: &str,
    pick: usize,
    cache_dir: &Path,
) -> Result<(Vec<u8>, String), WidgetError> {
    // Prefer matching the query against artist names first; theme queries
    // that match no artist fall back to full-text search.
    let mut ids: Vec<i64> = Vec::new();
    for artist_only in ["true", "false"] {
        let resp = http
            .get(&met.search_url)
            .query(&[
                ("q", query),
                ("hasImages", "true"),
                ("medium", "Paintings"),
                ("artistOrCulture", artist_only),
            ])
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await?
            .error_for_status()?;
        let parsed: SearchResponse = resp.json().await?;
        ids = parsed.object_ids.unwrap_or_default().into_iter().take(60).collect();
        if !ids.is_empty() {
            break;
        }
    }
    if ids.is_empty() {
        return Err(WidgetError::Data(format!("no artwork found for '{query}'")));
    }

    // Deterministic pick by day count; some objects lack a usable image or
    // aren't public domain, so walk forward until one qualifies.
    let attempts = ids.len().min(12);
    for offset in 0..attempts {
        let oid = ids[(pick + offset) % ids.len()];
        let cached_path = cache_dir.join(format!("met_{oid}.jpg"));
        let credit_path = cached_path.with_extension("txt");

        if cached_path.exists() {
            let credit = std::fs::read_to_string(&credit_path).unwrap_or_default();
            return Ok((std::fs::read(&cached_path)?, credit));
        }

        let object_url = met.object_url_template.replace("{}", &oid.to_string());
        let meta: ObjectResponse = http
            .get(&object_url)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let (Some(true), Some(image_url)) = (meta.is_public_domain, meta.primary_image_small.clone())
        else {
            continue;
        };
        let img_bytes = http
            .get(&image_url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec();
        std::fs::write(&cached_path, &img_bytes)?;
        let credit = [meta.title.as_deref(), meta.artist_display_name.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" \u{2014} ");
        std::fs::write(&credit_path, &credit)?;
        return Ok((img_bytes, credit));
    }
    Err(WidgetError::Data(format!(
        "no public-domain image among results for '{query}'"
    )))
}

/// Resolve the background art for `config`, either from a user-supplied
/// photo (no network) or by fetching/caching Met Museum artwork.
pub async fn fetch_art(
    http: &Client,
    met: &MetEndpoints,
    config: &CountdownConfig,
    today: NaiveDate,
) -> Result<CountdownArt, WidgetError> {
    if let Some(path) = &config.bg_photo {
        let bytes = std::fs::read(path)?;
        let mime = if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("png"))
            .unwrap_or(false)
        {
            "image/png"
        } else {
            "image/jpeg"
        };
        return Ok(CountdownArt {
            bg_uri: data_uri(mime, &bytes),
            credit: String::new(),
        });
    }

    let days = days_until(config.target_date, today);
    let pick = days.unsigned_abs() as usize;
    let (bytes, credit) =
        fetch_background(http, met, &config.bg_query, pick, &config.cache_dir).await?;
    Ok(CountdownArt {
        bg_uri: data_uri("image/jpeg", &bytes),
        credit,
    })
}

/// Pure render: template + config + resolved art + injected "today" -> HTML.
pub fn render_html(
    template_html: &str,
    config: &CountdownConfig,
    art: &CountdownArt,
    today: NaiveDate,
) -> Result<String, WidgetError> {
    let days = days_until(config.target_date, today);
    let label = countdown_label(days);
    let days_s = days.unsigned_abs().to_string();
    let date_s = config.target_date.format("%B %-d, %Y").to_string();

    Ok(template::render(
        template_html,
        &[
            ("BG_URI", art.bg_uri.as_str()),
            ("DAYS", days_s.as_str()),
            ("LABEL", label),
            ("TITLE", config.title.as_str()),
            ("DATE", date_s.as_str()),
            ("CREDIT", art.credit.as_str()),
        ],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{Path as AxPath, Query},
        routing::get,
        Json, Router,
    };
    use serde_json::json;
    use std::collections::HashMap;

    const TEMPLATE: &str = include_str!("../../templates/countdown-template.html");
    const PIXEL_JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9]; // minimal valid-enough JPEG marker bytes for tests

    fn tmp_cache_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bloomin8-countdown-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn days_until_is_positive_for_future_and_label_is_until() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 7).unwrap();
        let target = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        let days = days_until(target, today);
        assert_eq!(days, 177);
        assert_eq!(countdown_label(days), "days until");
    }

    #[test]
    fn days_until_is_negative_for_past_and_label_is_since() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 7).unwrap();
        let target = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let days = days_until(target, today);
        assert!(days < 0);
        assert_eq!(countdown_label(days), "days since");
    }

    #[test]
    fn render_html_has_no_leftover_placeholders() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 7).unwrap();
        let config = CountdownConfig {
            target_date: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            title: "New Year's Eve".to_string(),
            bg_photo: None,
            bg_query: "van gogh landscape".to_string(),
            cache_dir: tmp_cache_dir("render"),
        };
        let art = CountdownArt {
            bg_uri: data_uri("image/jpeg", PIXEL_JPEG),
            credit: "Starry Night \u{2014} Vincent van Gogh".to_string(),
        };
        let html = render_html(TEMPLATE, &config, &art, today).unwrap();
        assert!(!html.contains("{{"));
        assert!(html.contains("days until"));
        assert!(html.contains(">177<") || html.contains("177"));
    }

    #[tokio::test]
    async fn fetch_art_uses_user_supplied_photo_without_network() {
        let dir = tmp_cache_dir("photo");
        let photo_path = dir.join("me.jpg");
        std::fs::write(&photo_path, PIXEL_JPEG).unwrap();

        let config = CountdownConfig {
            target_date: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            title: "Anniversary".to_string(),
            bg_photo: Some(photo_path),
            bg_query: "unused".to_string(),
            cache_dir: dir,
        };
        let http = Client::new();
        // No mock server started: this must not hit the network at all.
        let met = MetEndpoints {
            search_url: "http://127.0.0.1:1/search".to_string(),
            object_url_template: "http://127.0.0.1:1/objects/{}".to_string(),
        };
        let art = fetch_art(&http, &met, &config, NaiveDate::from_ymd_opt(2026, 7, 7).unwrap())
            .await
            .unwrap();
        assert_eq!(art.credit, "");
        assert!(art.bg_uri.starts_with("data:image/jpeg;base64,"));
    }

    async fn start_mock_met(image_bytes: &'static [u8]) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let base_for_object = base.clone();

        let app = Router::new()
            .route(
                "/search",
                get(|Query(_q): Query<HashMap<String, String>>| async {
                    Json(json!({ "objectIDs": [42, 43, 44] }))
                }),
            )
            .route(
                "/objects/:id",
                get(move |AxPath(_id): AxPath<i64>| {
                    let base = base_for_object.clone();
                    async move {
                        Json(json!({
                            "isPublicDomain": true,
                            "primaryImageSmall": format!("{base}/image.jpg"),
                            "title": "Wheatfield with Crows",
                            "artistDisplayName": "Vincent van Gogh",
                        }))
                    }
                }),
            )
            .route("/image.jpg", get(move || async move { image_bytes }));

        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        base
    }

    #[tokio::test]
    async fn fetch_background_fetches_downloads_and_caches_artwork() {
        let base = start_mock_met(PIXEL_JPEG).await;
        let met = MetEndpoints {
            search_url: format!("{base}/search"),
            object_url_template: format!("{base}/objects/{{}}"),
        };
        let cache_dir = tmp_cache_dir("fetch");
        let http = Client::new();

        let (bytes, credit) = fetch_background(&http, &met, "van gogh", 0, &cache_dir)
            .await
            .expect("live fetch should succeed against the mock Met API");
        assert_eq!(bytes, PIXEL_JPEG);
        assert_eq!(credit, "Wheatfield with Crows \u{2014} Vincent van Gogh");
        // Second call for the same pick must hit the now-populated cache
        // (still succeeds, and returns the same cached bytes/credit).
        let (bytes2, credit2) = fetch_background(&http, &met, "van gogh", 0, &cache_dir)
            .await
            .expect("cached fetch should succeed");
        assert_eq!(bytes2, bytes);
        assert_eq!(credit2, credit);
    }

    #[tokio::test]
    async fn fetch_background_falls_back_to_cache_when_api_unreachable() {
        let cache_dir = tmp_cache_dir("fallback");
        // Pre-populate the cache as if a previous successful run had cached art.
        std::fs::write(cache_dir.join("met_100.jpg"), PIXEL_JPEG).unwrap();
        std::fs::write(cache_dir.join("met_200.jpg"), PIXEL_JPEG).unwrap();

        let met = MetEndpoints {
            search_url: "http://127.0.0.1:1/search".to_string(),
            object_url_template: "http://127.0.0.1:1/objects/{}".to_string(),
        };
        let http = Client::new();
        let (bytes, credit) = fetch_background(&http, &met, "anything", 3, &cache_dir)
            .await
            .expect("should fall back to cache when the API is unreachable");
        assert_eq!(bytes, PIXEL_JPEG);
        assert_eq!(credit, ""); // no sidecar .txt for pre-populated fixtures
    }

    #[tokio::test]
    async fn fetch_background_fails_when_api_unreachable_and_cache_empty() {
        let cache_dir = tmp_cache_dir("empty");
        let met = MetEndpoints {
            search_url: "http://127.0.0.1:1/search".to_string(),
            object_url_template: "http://127.0.0.1:1/objects/{}".to_string(),
        };
        let http = Client::new();
        let err = fetch_background(&http, &met, "anything", 0, &cache_dir)
            .await
            .unwrap_err();
        assert!(matches!(err, WidgetError::Data(_)));
    }

    #[test]
    fn date_format_strips_leading_zero_padding_like_pythons_percent_dash_d() {
        let d = NaiveDate::from_ymd_opt(2026, 7, 7).unwrap();
        assert_eq!(d.format("%B %-d, %Y").to_string(), "July 7, 2026");
    }
}

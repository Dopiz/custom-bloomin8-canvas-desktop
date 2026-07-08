//! Crypto price dashboard widget.
//!
//! Ports `~/.claude/skills/bloomin8-canvas/scripts/render.py` 1:1: Binance
//! public REST API (`/api/v3/ticker/24hr` + `/api/v3/klines`), mirror-host
//! fallback, bare-ticker `USDT` suffixing, 1/2-column grid density, and the
//! sparkline SVG (dashed range-open line + round HTML-overlay endpoint dot).
//!
//! Also ports `render.py`'s `icon_html()`: each coin's base symbol is looked
//! up as a PNG on a GitHub icon CDN, cached locally (`<base_lower>.png` under
//! a caller-supplied cache dir, mirroring countdown's injectable
//! `cache_dir`), and embedded as a base64 data URI. Unlike the market-data
//! fetcher's strict policy (any unreachable host fails the *whole* run), the
//! icon fetch is deliberately lenient: a network error, non-2xx response, or
//! io failure for one coin's icon only degrades that one card to the
//! lettered-circle fallback — it never fails [`render_html`] or the overall
//! render, matching `render.py`'s own "offline cron runs never break on a
//! missing icon" comment.

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use reqwest::Client;
use serde_json::Value;

use super::template;
use super::WidgetError;

pub const DEFAULT_HOSTS: &[&str] = &[
    "https://api.binance.com",
    "https://api1.binance.com",
    "https://api2.binance.com",
];

/// `{}` is replaced with the lowercased coin base symbol, e.g. `btc`.
pub const DEFAULT_ICON_CDN: &str =
    "https://raw.githubusercontent.com/spothq/cryptocurrency-icons/master/128/color/{}.png";

const QUOTE_SUFFIXES: &[&str] = &["USDT", "USDC", "FDUSD", "TUSD"];
const UP_COLOR: &str = "#009e3c";
const DOWN_COLOR: &str = "#e11900";

/// User-facing widget configuration.
#[derive(Debug, Clone)]
pub struct CryptoConfig {
    /// Raw symbols as typed by the user, e.g. `["BTC", "ETH", "SOLUSDC"]`.
    pub symbols: Vec<String>,
    /// One of `"24h"`, `"7d"`, `"30d"`.
    pub range: String,
    /// Directory used to cache fetched coin icon PNGs (`<base_lower>.png`),
    /// mirroring `render.py`'s `~/.cache/bloomin8-crypto-icons/`. Injectable
    /// so tests can point it at a temp dir instead of the real home cache.
    pub icon_cache_dir: PathBuf,
}

/// Market data for one (normalized) symbol, as returned by the fetcher and
/// consumed by the pure renderer.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolMarket {
    /// Normalized symbol, e.g. `BTCUSDT`.
    pub symbol: String,
    pub price: f64,
    pub change_percent: f64,
    /// Close prices for the chosen range's klines, oldest to newest.
    pub closes: Vec<f64>,
    /// Opening price of the first kline in the range (the sparkline's
    /// dashed 0%-change reference line).
    pub range_open: f64,
}

/// `interval, limit` for each supported range (mirrors `RANGES` in
/// `render.py`).
fn range_params(range: &str) -> Result<(&'static str, u32), WidgetError> {
    match range {
        "24h" => Ok(("1h", 24)),
        "7d" => Ok(("4h", 42)),
        "30d" => Ok(("1d", 30)),
        other => Err(WidgetError::Data(format!("unsupported crypto range: {other}"))),
    }
}

pub fn normalize_symbol(sym: &str) -> String {
    let sym = sym.trim().to_uppercase();
    if QUOTE_SUFFIXES.iter().any(|suf| sym.ends_with(suf)) {
        sym
    } else {
        format!("{sym}USDT")
    }
}

/// Splits a normalized symbol into (base, quote), e.g. `BTCUSDT` -> `(BTC,
/// USDT)`. Assumes `symbol` was produced by [`normalize_symbol`].
fn split_symbol(symbol: &str) -> (&str, &str) {
    for suf in QUOTE_SUFFIXES {
        if symbol.ends_with(suf) {
            return (&symbol[..symbol.len() - suf.len()], suf);
        }
    }
    (symbol, "")
}

fn group_thousands(int_part: &str) -> String {
    let bytes = int_part.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Port of `fmt_price` in `render.py`: `>=1000` -> grouped, no decimals;
/// `>=1` -> grouped, 2 decimals; else 4 decimals, no grouping.
pub fn fmt_price(p: f64) -> String {
    if p >= 1000.0 {
        group_thousands(&format!("{:.0}", p))
    } else if p >= 1.0 {
        let s = format!("{:.2}", p);
        let (int_part, frac_part) = s.split_once('.').unwrap_or((s.as_str(), ""));
        format!("{}.{}", group_thousands(int_part), frac_part)
    } else {
        format!("{p:.4}")
    }
}

/// Port of `sparkline_chart` in `render.py`.
pub fn sparkline_chart(closes: &[f64], baseline: f64, accent: &str) -> String {
    sparkline_chart_sized(closes, baseline, accent, 560.0, 160.0)
}

fn sparkline_chart_sized(closes: &[f64], baseline: f64, accent: &str, w: f64, h: f64) -> String {
    let lo = closes.iter().cloned().fold(baseline, f64::min);
    let hi = closes.iter().cloned().fold(baseline, f64::max);
    let span = if hi - lo == 0.0 { 1.0 } else { hi - lo };
    let pad = 8.0_f64;
    let n = closes.len();
    let pts: Vec<(f64, f64)> = closes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let x = pad + i as f64 * (w - 2.0 * pad) / (n as f64 - 1.0);
            let y = pad + (hi - c) * (h - 2.0 * pad) / span;
            (x, y)
        })
        .collect();
    let line = pts
        .iter()
        .map(|(x, y)| format!("{x:.1},{y:.1}"))
        .collect::<Vec<_>>()
        .join(" ");
    let area = format!(
        "{:.1},{:.1} {} {:.1},{:.1}",
        pad,
        h - pad,
        line,
        w - pad,
        h - pad
    );
    let by = pad + (hi - baseline) * (h - 2.0 * pad) / span;
    let (lx, ly) = *pts.last().expect("closes must be non-empty");
    format!(
        "<svg viewBox=\"0 0 {w} {h}\" preserveAspectRatio=\"none\" xmlns=\"http://www.w3.org/2000/svg\">\
<polygon points=\"{area}\" fill=\"#e0e0e0\"/>\
<line x1=\"{pad}\" y1=\"{by:.1}\" x2=\"{}\" y2=\"{by:.1}\" stroke=\"#000\" stroke-width=\"2\" stroke-dasharray=\"8 8\" vector-effect=\"non-scaling-stroke\"/>\
<polyline points=\"{line}\" fill=\"none\" stroke=\"#000\" stroke-width=\"3.5\" stroke-linejoin=\"round\" stroke-linecap=\"round\" vector-effect=\"non-scaling-stroke\"/>\
</svg>\
<span class=\"dot\" style=\"left:{:.2}%;top:{:.2}%;background:{accent}\"></span>",
        w - pad,
        lx / w * 100.0,
        ly / h * 100.0,
    )
}

/// Lettered-circle fallback markup, mirrors `render.py`'s
/// `<div class="icon fallback">{base[0]}</div>`. Public so deterministic
/// fixtures (golden tests) can render icons without any CDN/cache access.
pub fn icon_fallback_html(base: &str) -> String {
    format!(
        "<div class=\"icon fallback\">{}</div>",
        base.chars().next().unwrap_or('?')
    )
}

/// Embeds already-fetched PNG bytes as a base64 `<img>`, mirrors
/// `render.py`'s `<img class="icon" src="data:image/png;base64,...">`.
fn icon_embed_html(bytes: &[u8]) -> String {
    let uri = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );
    format!("<img class=\"icon\" src=\"{uri}\">")
}

/// Resolves one coin's icon markup (port of `render.py`'s `icon_html()`):
/// cache hit -> embed cached bytes; cache miss -> GET `icon_cdn` (with `{}`
/// replaced by the lowercased `base`), cache the PNG on success, then embed;
/// any failure (network error, non-2xx, cache dir unwritable) -> lettered
/// circle fallback. This function itself never fails/panics — the lenient
/// per-icon policy is baked in, so callers don't need to handle an `Err`.
pub async fn fetch_icon_html(http: &Client, icon_cdn: &str, cache_dir: &Path, base: &str) -> String {
    let base_lower = base.to_lowercase();
    let cached_path = cache_dir.join(format!("{base_lower}.png"));

    if let Ok(bytes) = std::fs::read(&cached_path) {
        return icon_embed_html(&bytes);
    }

    if std::fs::create_dir_all(cache_dir).is_err() {
        return icon_fallback_html(base);
    }

    let url = icon_cdn.replace("{}", &base_lower);
    let fetched = async {
        let resp = http
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?;
        resp.bytes().await
    }
    .await;

    match fetched {
        Ok(bytes) => {
            // Best-effort write: even if caching fails, we still have the
            // bytes in hand for this render.
            let _ = std::fs::write(&cached_path, &bytes);
            icon_embed_html(&bytes)
        }
        Err(_) => icon_fallback_html(base),
    }
}

/// Resolves icon markup for every market, aligned by index with `markets`.
/// Never fails: each coin independently falls back to the lettered circle
/// per [`fetch_icon_html`]'s policy.
pub async fn fetch_icons(
    http: &Client,
    icon_cdn: &str,
    cache_dir: &Path,
    markets: &[SymbolMarket],
) -> Vec<String> {
    let mut out = Vec::with_capacity(markets.len());
    for market in markets {
        let (base, _quote) = split_symbol(&market.symbol);
        out.push(fetch_icon_html(http, icon_cdn, cache_dir, base).await);
    }
    out
}

fn build_card(market: &SymbolMarket, icon_html: &str) -> String {
    let up = market.change_percent >= 0.0;
    let accent = if up { UP_COLOR } else { DOWN_COLOR };
    let (base, quote) = split_symbol(&market.symbol);
    let lo = market.closes.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = market
        .closes
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    format!(
        "    <div class=\"card\"><div class=\"card-inner\">\n      \
<div class=\"card-head\">\n        \
<div class=\"symbol-wrap\">{icon_html}<div class=\"symbol\">{base} <small>/ {quote}</small></div></div>\n        \
<div class=\"badge {updown}\">{arrow} {change:.2}%<span class=\"tf\">24H</span></div>\n      \
</div>\n      \
<div class=\"price\"><span class=\"cur\">$</span>{price}</div>\n      \
<div class=\"chart\">{chart}</div>\n      \
<div class=\"range-row\"><span>L ${lo}</span><span>H ${hi}</span></div>\n    \
</div></div>",
        updown = if up { "up" } else { "down" },
        arrow = if up { "\u{25b2}" } else { "\u{25bc}" },
        change = market.change_percent.abs(),
        price = fmt_price(market.price),
        chart = sparkline_chart(&market.closes, market.range_open, accent),
        lo = fmt_price(lo),
        hi = fmt_price(hi),
    )
}

async fn get_with_fallback(
    http: &Client,
    hosts: &[String],
    path: &str,
    query: &[(&str, String)],
) -> Result<Value, WidgetError> {
    let mut last_err: Option<WidgetError> = None;
    for host in hosts {
        let url = format!("{host}{path}");
        let result = async {
            let resp = http
                .get(&url)
                .query(query)
                .timeout(Duration::from_secs(10))
                .send()
                .await?
                .error_for_status()?;
            Ok::<Value, reqwest::Error>(resp.json::<Value>().await?)
        }
        .await;
        match result {
            Ok(v) => return Ok(v),
            Err(e) => last_err = Some(WidgetError::from(e)),
        }
    }
    Err(last_err.unwrap_or_else(|| WidgetError::Data(format!("no hosts configured for {path}"))))
}

/// Fetch market data for every configured symbol against the real Binance
/// hosts (mirror fallback per `DEFAULT_HOSTS`).
pub async fn fetch(http: &Client, config: &CryptoConfig) -> Result<Vec<SymbolMarket>, WidgetError> {
    let hosts: Vec<String> = DEFAULT_HOSTS.iter().map(|s| s.to_string()).collect();
    fetch_with_hosts(http, config, &hosts).await
}

/// Same as [`fetch`] but with an injectable host list, so tests can point at
/// a mock server instead of the real Binance API.
pub async fn fetch_with_hosts(
    http: &Client,
    config: &CryptoConfig,
    hosts: &[String],
) -> Result<Vec<SymbolMarket>, WidgetError> {
    if config.symbols.is_empty() {
        return Err(WidgetError::Data("no symbols given".to_string()));
    }
    let (interval, limit) = range_params(&config.range)?;
    let mut out = Vec::with_capacity(config.symbols.len());
    for raw in &config.symbols {
        let symbol = normalize_symbol(raw);
        let ticker = get_with_fallback(
            http,
            hosts,
            "/api/v3/ticker/24hr",
            &[("symbol", symbol.clone())],
        )
        .await?;
        let klines = get_with_fallback(
            http,
            hosts,
            "/api/v3/klines",
            &[
                ("symbol", symbol.clone()),
                ("interval", interval.to_string()),
                ("limit", limit.to_string()),
            ],
        )
        .await?;

        let price = ticker
            .get("lastPrice")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| WidgetError::Data(format!("{symbol}: missing lastPrice in ticker")))?;
        let change_percent = ticker
            .get("priceChangePercent")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| {
                WidgetError::Data(format!("{symbol}: missing priceChangePercent in ticker"))
            })?;

        let klines_arr = klines
            .as_array()
            .ok_or_else(|| WidgetError::Data(format!("{symbol}: klines response is not an array")))?;
        if klines_arr.is_empty() {
            return Err(WidgetError::Data(format!("{symbol}: empty klines response")));
        }
        let closes: Vec<f64> = klines_arr
            .iter()
            .map(|k| {
                k.get(4)
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<f64>().ok())
                    .ok_or_else(|| WidgetError::Data(format!("{symbol}: malformed kline entry")))
            })
            .collect::<Result<_, _>>()?;
        let range_open = klines_arr[0]
            .get(1)
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| WidgetError::Data(format!("{symbol}: malformed kline open")))?;

        out.push(SymbolMarket {
            symbol,
            price,
            change_percent,
            closes,
            range_open,
        });
    }
    Ok(out)
}

/// Pure render: template + config + fetched data + resolved icon markup +
/// injected "now" -> HTML. `icons` must be the same length as `markets` and
/// aligned by index (see [`fetch_icons`]).
pub fn render_html(
    template_html: &str,
    config: &CryptoConfig,
    markets: &[SymbolMarket],
    icons: &[String],
    now: chrono::NaiveDateTime,
) -> Result<String, WidgetError> {
    if markets.is_empty() {
        return Err(WidgetError::Data("no symbols given".to_string()));
    }
    if icons.len() != markets.len() {
        return Err(WidgetError::Data(
            "icons and markets must have the same length".to_string(),
        ));
    }
    let n = markets.len();
    // Grid density: up to 3 cards stack in a single line; beyond that, wrap
    // into 2 rows/columns (mirrors render.py's cols_portrait/cols_landscape).
    let cols_portrait: usize = if n <= 3 { 1 } else { 2 };
    let cols_landscape: usize = if n <= 3 { n } else { (n + 1) / 2 };
    let cards = markets
        .iter()
        .zip(icons.iter())
        .map(|(market, icon_html)| build_card(market, icon_html))
        .collect::<Vec<_>>()
        .join("\n");

    let cols_portrait_s = cols_portrait.to_string();
    let cols_landscape_s = cols_landscape.to_string();
    let range_label = config.range.to_uppercase();
    let updated_at = now.format("%Y-%m-%d %H:%M").to_string();

    Ok(template::render(
        template_html,
        &[
            ("CARDS", cards.as_str()),
            ("COLS_PORTRAIT", cols_portrait_s.as_str()),
            ("COLS_LANDSCAPE", cols_landscape_s.as_str()),
            ("RANGE_LABEL", range_label.as_str()),
            ("UPDATED_AT", updated_at.as_str()),
        ],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Query, routing::get, Json, Router};
    use serde::Deserialize;
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    const TEMPLATE: &str = include_str!("../../templates/crypto-template.html");
    // Minimal-but-valid 1x1 PNG, used as a stand-in for a real coin icon.
    const PIXEL_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn sample_now() -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 7)
            .unwrap()
            .and_hms_opt(9, 30, 0)
            .unwrap()
    }

    fn tmp_cache_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bloomin8-crypto-icon-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Icons aligned with `markets` for tests that don't exercise icon
    /// fetching itself, just render plumbing.
    fn fallback_icons(markets: &[SymbolMarket]) -> Vec<String> {
        markets
            .iter()
            .map(|m| icon_fallback_html(split_symbol(&m.symbol).0))
            .collect()
    }

    #[test]
    fn normalize_symbol_appends_usdt_only_when_no_quote_suffix() {
        assert_eq!(normalize_symbol("btc"), "BTCUSDT");
        assert_eq!(normalize_symbol(" eth "), "ETHUSDT");
        assert_eq!(normalize_symbol("solusdc"), "SOLUSDC");
        assert_eq!(normalize_symbol("BTCFDUSD"), "BTCFDUSD");
    }

    #[test]
    fn fmt_price_matches_python_thresholds() {
        assert_eq!(fmt_price(65000.4), "65,000");
        assert_eq!(fmt_price(1234.5), "1,234"); // >=1000 rounds (half-to-even), no decimals
        assert_eq!(fmt_price(3.14159), "3.14");
        assert_eq!(fmt_price(0.00012345), "0.0001");
    }

    #[test]
    fn sparkline_ends_with_positioned_dot_and_dashed_baseline() {
        let svg = sparkline_chart(&[10.0, 12.0, 9.0, 15.0], 10.0, "#009e3c");
        assert!(svg.contains("stroke-dasharray=\"8 8\""));
        assert!(svg.contains("class=\"dot\""));
        assert!(svg.starts_with("<svg"));
    }

    fn mock_market(symbol: &str) -> SymbolMarket {
        SymbolMarket {
            symbol: symbol.to_string(),
            price: 65000.0,
            change_percent: 1.23,
            closes: vec![64000.0, 64500.0, 65000.0],
            range_open: 64000.0,
        }
    }

    #[test]
    fn render_html_grid_class_is_1_col_for_1_to_3_symbols() {
        let cfg = CryptoConfig {
            symbols: vec!["BTC".into()],
            range: "24h".into(),
            icon_cache_dir: tmp_cache_dir("grid1"),
        };
        let markets = vec![mock_market("BTCUSDT")];
        let icons = fallback_icons(&markets);
        let html = render_html(TEMPLATE, &cfg, &markets, &icons, sample_now()).unwrap();
        assert!(html.contains("repeat(1, 1fr)"));
    }

    #[test]
    fn render_html_grid_class_is_1_col_for_3_symbols() {
        let cfg = CryptoConfig {
            symbols: vec!["BTC".into(), "ETH".into(), "SOL".into()],
            range: "24h".into(),
            icon_cache_dir: tmp_cache_dir("grid3"),
        };
        let markets = vec![
            mock_market("BTCUSDT"),
            mock_market("ETHUSDT"),
            mock_market("SOLUSDT"),
        ];
        let icons = fallback_icons(&markets);
        let html = render_html(TEMPLATE, &cfg, &markets, &icons, sample_now()).unwrap();
        assert!(html.contains("repeat(1, 1fr)"));
        assert!(html.contains("repeat(3, 1fr)")); // landscape cols == n when n<=3
    }

    #[test]
    fn render_html_grid_class_is_2_col_for_4_symbols() {
        let cfg = CryptoConfig {
            symbols: vec!["BTC".into(), "ETH".into(), "SOL".into(), "XRP".into()],
            range: "24h".into(),
            icon_cache_dir: tmp_cache_dir("grid4"),
        };
        let markets = vec![
            mock_market("BTCUSDT"),
            mock_market("ETHUSDT"),
            mock_market("SOLUSDT"),
            mock_market("XRPUSDT"),
        ];
        let icons = fallback_icons(&markets);
        let html = render_html(TEMPLATE, &cfg, &markets, &icons, sample_now()).unwrap();
        assert!(html.contains("repeat(2, 1fr)"));
    }

    #[derive(Deserialize)]
    struct TickerQuery {
        symbol: String,
    }

    #[derive(Deserialize)]
    struct KlinesQuery {
        symbol: String,
        #[allow(dead_code)]
        interval: String,
        #[allow(dead_code)]
        limit: u32,
    }

    async fn start_mock_binance(hit_counter: Arc<AtomicUsize>) -> String {
        let hits = hit_counter.clone();
        let ticker = get(move |Query(q): Query<TickerQuery>| {
            let hits = hits.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "symbol": q.symbol,
                    "lastPrice": "65000.12",
                    "priceChangePercent": "2.50",
                }))
            }
        });
        let klines = get(|Query(_q): Query<KlinesQuery>| async move {
            Json(json!([
                ["_", "64000.0", "_", "_", "64500.0"],
                ["_", "64000.0", "_", "_", "65000.0"],
                ["_", "64000.0", "_", "_", "65500.0"],
            ]))
        });
        let app = Router::new()
            .route("/api/v3/ticker/24hr", ticker)
            .route("/api/v3/klines", klines);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn fetch_with_hosts_parses_ticker_and_klines() {
        let base = start_mock_binance(Arc::new(AtomicUsize::new(0))).await;
        let http = Client::new();
        let cfg = CryptoConfig {
            symbols: vec!["btc".into()],
            range: "24h".into(),
            icon_cache_dir: tmp_cache_dir("fetch-parse"),
        };
        let markets = fetch_with_hosts(&http, &cfg, &[base]).await.unwrap();
        assert_eq!(markets.len(), 1);
        assert_eq!(markets[0].symbol, "BTCUSDT");
        assert_eq!(markets[0].price, 65000.12);
        assert_eq!(markets[0].change_percent, 2.50);
        assert_eq!(markets[0].closes, vec![64500.0, 65000.0, 65500.0]);
        assert_eq!(markets[0].range_open, 64000.0);
    }

    #[tokio::test]
    async fn fetch_with_hosts_falls_through_to_a_working_mirror() {
        let good_base = start_mock_binance(Arc::new(AtomicUsize::new(0))).await;
        // A host with nothing listening simulates a dead mirror: connection
        // refused, so the fallback loop should move on to `good_base`.
        let dead_host = "http://127.0.0.1:1".to_string();

        let http = Client::new();
        let cfg = CryptoConfig {
            symbols: vec!["ETH".into()],
            range: "24h".into(),
            icon_cache_dir: tmp_cache_dir("fetch-mirror"),
        };
        let markets = fetch_with_hosts(&http, &cfg, &[dead_host, good_base])
            .await
            .unwrap();
        assert_eq!(markets[0].symbol, "ETHUSDT");
    }

    #[tokio::test]
    async fn fetch_with_hosts_fails_when_every_host_is_unreachable() {
        let http = Client::new();
        let cfg = CryptoConfig {
            symbols: vec!["BTC".into()],
            range: "24h".into(),
            icon_cache_dir: tmp_cache_dir("fetch-unreachable"),
        };
        let err = fetch_with_hosts(
            &http,
            &cfg,
            &["http://127.0.0.1:1".to_string(), "http://127.0.0.1:2".to_string()],
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WidgetError::Http(_)));
    }

    async fn start_mock_icon_cdn(png_bytes: &'static [u8]) -> String {
        let app = Router::new().route(
            "/:sym",
            get(move || async move { png_bytes }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        format!("http://{addr}/{{}}.png")
    }

    async fn start_mock_icon_cdn_404() -> String {
        let app = Router::new().route(
            "/:sym",
            get(|| async { (axum::http::StatusCode::NOT_FOUND, "not found") }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        format!("http://{addr}/{{}}.png")
    }

    #[tokio::test]
    async fn fetch_icon_html_embeds_base64_img_on_cdn_success() {
        let cdn = start_mock_icon_cdn(PIXEL_PNG).await;
        let cache_dir = tmp_cache_dir("icon-success");
        let http = Client::new();

        let html = fetch_icon_html(&http, &cdn, &cache_dir, "BTC").await;
        assert!(html.starts_with("<img class=\"icon\" src=\"data:image/png;base64,"));
        assert!(!html.contains("fallback"));
        // Successful fetch must populate the on-disk cache.
        assert!(cache_dir.join("btc.png").exists());
    }

    #[tokio::test]
    async fn fetch_icon_html_falls_back_to_lettered_circle_on_404() {
        let cdn = start_mock_icon_cdn_404().await;
        let cache_dir = tmp_cache_dir("icon-404");
        let http = Client::new();

        let html = fetch_icon_html(&http, &cdn, &cache_dir, "ETH").await;
        assert_eq!(html, "<div class=\"icon fallback\">E</div>");
        assert!(!cache_dir.join("eth.png").exists());
    }

    #[tokio::test]
    async fn render_html_never_fails_when_an_icon_404s() {
        // The whole render must stay Ok even though one coin's icon 404s,
        // per the lenient per-icon fallback policy.
        let cdn = start_mock_icon_cdn_404().await;
        let cache_dir = tmp_cache_dir("icon-404-render");
        let http = Client::new();
        let markets = vec![mock_market("BTCUSDT")];
        let icons = fetch_icons(&http, &cdn, &cache_dir, &markets).await;
        assert_eq!(icons, vec!["<div class=\"icon fallback\">B</div>".to_string()]);

        let cfg = CryptoConfig {
            symbols: vec!["BTC".into()],
            range: "24h".into(),
            icon_cache_dir: cache_dir,
        };
        let html = render_html(TEMPLATE, &cfg, &markets, &icons, sample_now()).unwrap();
        assert!(html.contains("icon fallback"));
        assert!(!html.contains("<img class=\"icon\""));
    }

    #[tokio::test]
    async fn fetch_icon_html_reads_from_cache_without_hitting_the_network() {
        let cache_dir = tmp_cache_dir("icon-cache-hit");
        std::fs::write(cache_dir.join("sol.png"), PIXEL_PNG).unwrap();

        // No mock server started: an unreachable CDN URL proves the cache
        // hit path never makes a network request.
        let dead_cdn = "http://127.0.0.1:1/{}.png".to_string();
        let http = Client::new();

        let html = fetch_icon_html(&http, &dead_cdn, &cache_dir, "SOL").await;
        assert!(html.starts_with("<img class=\"icon\" src=\"data:image/png;base64,"));
        let expected_uri = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(PIXEL_PNG)
        );
        assert_eq!(html, format!("<img class=\"icon\" src=\"{expected_uri}\">"));
    }
}

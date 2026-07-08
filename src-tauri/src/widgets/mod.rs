//! Widget data fetchers + HTML renderers.
//!
//! Ports three Python reference scripts 1:1 in semantics:
//! `~/.claude/skills/bloomin8-canvas/scripts/{render,weather,countdown}.py`.
//!
//! Every widget separates **fetching** (hits a real API, returns a typed
//! struct) from **rendering** (a pure function: struct + injected "now" ->
//! HTML string), so rendering can be unit tested without any network or
//! wall-clock dependency.
//!
//! Failure policy: if a widget's data API is unreachable after
//! trying all documented fallbacks, the whole fetch fails (`Err`) — widgets
//! never render stale or fabricated data. The one documented exception is
//! countdown's artwork cache: a *cached* image
//! is not "stale/fabricated" data, it's the explicitly-specified offline
//! fallback, so it still counts as success.
//!
//! The render pipeline consumes the pure `render_html` functions (via
//! `crate::fixtures`); the fetchers stay unwired until the Widgets page,
//! so parts may still look "unused" from the compiler's point of view.
#![allow(dead_code)]

pub mod config;
pub mod countdown;
pub mod crypto;
pub mod template;
pub mod weather;

use thiserror::Error;

/// Shared error type for all widget fetchers/renderers.
#[derive(Debug, Error)]
pub enum WidgetError {
    #[error("http request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("failed to parse API response: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("template error: {0}")]
    Template(#[from] template::TemplateError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Data(String),
}

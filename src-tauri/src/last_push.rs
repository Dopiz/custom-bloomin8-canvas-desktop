//! Remembers the last image pushed to each device — its filename and the
//! orientation it was pushed with — in `last_push.json` in the app data dir.
//!
//! Why this exists: the panel is physically portrait (1200x1600) and a
//! landscape push is rotated 90° into that portrait file (see
//! `commands::process_push_image`), so on the device a landscape image is
//! indistinguishable from a portrait one by pixels alone. The Device page
//! hero needs to know a shown image was pushed as landscape to display it
//! upright, so we record the orientation at push time and look it up by the
//! filename the device reports as currently shown.
//!
//! Kept deliberately best-effort: a failed `record` only logs and never fails
//! the push it hangs off of.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

pub const LAST_PUSH_FILE_NAME: &str = "last_push.json";

/// One device's last push: the exact filename the device reported showing and
/// the orientation it was pushed with (`"portrait"` | `"landscape"`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LastPush {
    pub filename: String,
    pub orientation: String,
}

fn store_path(app: &AppHandle) -> Option<PathBuf> {
    match app.path().app_data_dir() {
        Ok(dir) => Some(dir.join(LAST_PUSH_FILE_NAME)),
        Err(e) => {
            eprintln!("[last_push] could not resolve app data dir: {e}");
            None
        }
    }
}

/// Load the `{ device_id -> LastPush }` map, tolerating a missing or corrupt
/// file by returning an empty map (this is a best-effort convenience store,
/// never a source of hard errors).
fn load_map(path: &Path) -> HashMap<String, LastPush> {
    match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Record (overwrite) the last push for `device_id`. Best-effort: any failure
/// is logged and swallowed so a push never fails just because this bookkeeping
/// couldn't be written.
pub(crate) fn record(app: &AppHandle, device_id: &str, filename: &str, orientation: &str) {
    let Some(path) = store_path(app) else {
        return;
    };
    let mut map = load_map(&path);
    map.insert(
        device_id.to_string(),
        LastPush {
            filename: filename.to_string(),
            orientation: orientation.to_string(),
        },
    );
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(&map) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!("[last_push] failed to write {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("[last_push] failed to serialize store: {e}"),
    }
}

/// Forget a device's last-push record (used when a device is removed).
/// Best-effort: failures are logged, never propagated.
pub(crate) fn remove(app: &AppHandle, device_id: &str) {
    let Some(path) = store_path(app) else {
        return;
    };
    let mut map = load_map(&path);
    if map.remove(device_id).is_none() {
        return; // nothing to write back
    }
    match serde_json::to_string_pretty(&map) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!("[last_push] failed to write {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("[last_push] failed to serialize store: {e}"),
    }
}

/// The last push recorded for the *currently active* device (resolved via
/// `device.json`), or `None` if nothing has been pushed to it yet.
#[tauri::command]
pub fn last_push(app: AppHandle) -> Option<LastPush> {
    let cfg_path = app
        .path()
        .app_data_dir()
        .ok()?
        .join(crate::config::CONFIG_FILE_NAME);
    let cfg = crate::config::load(&cfg_path).ok()?;
    let device_id = cfg.active_device()?.id.clone();
    let path = store_path(&app)?;
    load_map(&path).get(&device_id).cloned()
}

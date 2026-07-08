//! Local image library: the user's own uploaded originals, kept on disk in the
//! app data dir. This is distinct from what's actually on the device — an
//! upload here does NOT push to the panel; the user pushes a library image
//! (with per-push display settings, see `commands::push_image`) when they want
//! it on the Canvas, and the original always stays here.
//!
//! Storage: `<app_data>/library/<id>.<ext>` for the bytes, plus an
//! `index.json` listing `{id, name, added, ext}` newest-first.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryItem {
    pub id: String,
    /// Original filename the user picked (display only).
    pub name: String,
    /// Millis since epoch when added.
    pub added: u64,
    /// File extension of the stored original (jpg/png/webp/…).
    pub ext: String,
}

fn library_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve app data directory: {e}"))?
        .join("library");
    Ok(dir)
}

fn index_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(library_dir(app)?.join("index.json"))
}

fn read_index(app: &AppHandle) -> Result<Vec<LibraryItem>, String> {
    let path = index_path(app)?;
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("corrupt library index: {e}")),
        Err(_) => Ok(Vec::new()), // no library yet
    }
}

fn write_index(app: &AppHandle, items: &[LibraryItem]) -> Result<(), String> {
    let dir = library_dir(app)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create library dir: {e}"))?;
    let json = serde_json::to_vec_pretty(items).map_err(|e| e.to_string())?;
    std::fs::write(index_path(app)?, json).map_err(|e| format!("could not write index: {e}"))
}

/// Decode a `data:<mime>;base64,<...>` URL into (bytes, extension).
fn decode_data_url(data_url: &str) -> Result<(Vec<u8>, String), String> {
    let mime = data_url
        .strip_prefix("data:")
        .and_then(|s| s.split(';').next())
        .unwrap_or("");
    let ext = match mime {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        _ => "jpg",
    }
    .to_string();
    let b64 = data_url.rsplit(',').next().unwrap_or("");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("could not decode image data: {e}"))?;
    Ok((bytes, ext))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn item_file(app: &AppHandle, item: &LibraryItem) -> Result<PathBuf, String> {
    Ok(library_dir(app)?.join(format!("{}.{}", item.id, item.ext)))
}

fn mime_for(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tiff" => "image/tiff",
        _ => "image/jpeg",
    }
}

/// List library items, newest first.
#[tauri::command]
pub fn library_list(app: AppHandle) -> Result<Vec<LibraryItem>, String> {
    read_index(&app)
}

/// Save an uploaded original into the library. `data_url` is the file's
/// `data:...;base64,...` (from a browser file `<input>`). Returns the new item.
#[tauri::command]
pub fn library_add(app: AppHandle, name: String, data_url: String) -> Result<LibraryItem, String> {
    let (bytes, ext) = decode_data_url(&data_url)?;
    let item = LibraryItem {
        id: new_id(),
        name: if name.trim().is_empty() { "image".to_string() } else { name },
        added: now_ms(),
        ext,
    };
    let dir = library_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create library dir: {e}"))?;
    std::fs::write(item_file(&app, &item)?, &bytes)
        .map_err(|e| format!("could not save image: {e}"))?;
    let mut items = read_index(&app)?;
    items.insert(0, item.clone());
    write_index(&app, &items)?;
    Ok(item)
}

/// Delete a library item (its file + index entry). Does not touch the device.
#[tauri::command]
pub fn library_delete(app: AppHandle, id: String) -> Result<(), String> {
    let mut items = read_index(&app)?;
    if let Some(pos) = items.iter().position(|i| i.id == id) {
        let item = items.remove(pos);
        let _ = std::fs::remove_file(item_file(&app, &item)?);
        write_index(&app, &items)?;
    }
    Ok(())
}

/// Read a library image's original bytes by id — used by the scheduler's
/// image-schedule push (it processes these bytes through
/// `commands::process_push_image` instead of the widget render pipeline). A
/// missing id or a deleted-on-disk original is a clear error string, so an
/// orphaned image schedule surfaces a `Failed` run rather than panicking.
pub(crate) fn read_image_bytes(app: &AppHandle, id: &str) -> Result<Vec<u8>, String> {
    let items = read_index(app)?;
    let item = items
        .iter()
        .find(|i| i.id == id)
        .ok_or_else(|| format!("image {id:?} is no longer in the library"))?;
    std::fs::read(item_file(app, item)?).map_err(|e| format!("could not read image: {e}"))
}

/// Return a library image's original bytes as a `data:<mime>;base64,...` URL —
/// used both to render the thumbnail and as the source for preview/push.
#[tauri::command]
pub fn library_image(app: AppHandle, id: String) -> Result<String, String> {
    let items = read_index(&app)?;
    let item = items
        .iter()
        .find(|i| i.id == id)
        .ok_or_else(|| "image not found in library".to_string())?;
    let bytes =
        std::fs::read(item_file(&app, item)?).map_err(|e| format!("could not read image: {e}"))?;
    Ok(format!(
        "data:{};base64,{}",
        mime_for(&item.ext),
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

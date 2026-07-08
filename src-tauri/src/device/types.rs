use serde::{Deserialize, Serialize};

/// `GET /deviceInfo` response.
///
/// The real firmware returns more fields than we currently care about; unknown
/// fields are captured in `extra` instead of failing deserialization so a
/// firmware update that adds a field doesn't break this client.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct DeviceInfo {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub battery: Option<i64>,
    /// Path of the image currently displayed on the panel, e.g.
    /// `/gallerys/default/frame_20260101_120000.jpg`.
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub gallery: Option<String>,
    #[serde(default)]
    pub max_idle: Option<u64>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// `GET /state` response. `status == 100` means Ready.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct DeviceState {
    pub status: i64,
    #[serde(default)]
    pub msg: String,
}

/// The firmware's "Ready" status code.
pub const STATUS_READY: i64 = 100;

impl DeviceState {
    pub fn is_ready(&self) -> bool {
        self.status == STATUS_READY
    }
}

/// Body for `POST /settings`. Any subset of fields may be set; unset fields
/// are omitted from the JSON body entirely rather than sent as `null`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceSettingsUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_idle: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idx_wake_sens: Option<u64>,
}

/// A single image entry as returned by `GET /gallery` (paginated per-gallery
/// image listing).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GalleryImage {
    pub name: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Result of a successful [`crate::device::DeviceClient::upload_and_show`]
/// call: the fresh filename that was uploaded and verified as displayed.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadAndShowResult {
    pub filename: String,
    pub gallery: String,
}

/// An entry in `GET /gallery/list` (the list of galleries, as opposed to
/// [`GalleryImage`] which lists a single gallery's images).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GallerySummary {
    pub name: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// An entry in `GET /playlist/list`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PlaylistSummary {
    pub name: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// `play_type` values accepted by `POST /show` ("Show single image
/// / gallery slideshow / playlist"). Represented as an enum rather than a
/// bare integer so callers can't pass an invalid play type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayType {
    Image = 0,
    Gallery = 1,
    Playlist = 2,
}

impl PlayType {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Body for `POST /show`, matching `client.py`'s `show-image` /
/// `show-gallery` / `show-playlist` subcommands exactly (each carries a
/// different set of fields alongside `play_type`).
#[derive(Debug, Clone, PartialEq)]
pub enum ShowRequest {
    /// `{"play_type": 0, "image": <device path>}`
    Image { image: String },
    /// `{"play_type": 1, "gallery": <name>, "duration": <seconds per image>}`
    Gallery { gallery: String, duration: u32 },
    /// `{"play_type": 2, "playlist": <name>}`
    Playlist { playlist: String },
}

impl ShowRequest {
    pub fn play_type(&self) -> PlayType {
        match self {
            ShowRequest::Image { .. } => PlayType::Image,
            ShowRequest::Gallery { .. } => PlayType::Gallery,
            ShowRequest::Playlist { .. } => PlayType::Playlist,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            ShowRequest::Image { image } => serde_json::json!({
                "play_type": self.play_type().as_u8(),
                "image": image,
            }),
            ShowRequest::Gallery { gallery, duration } => serde_json::json!({
                "play_type": self.play_type().as_u8(),
                "gallery": gallery,
                "duration": duration,
            }),
            ShowRequest::Playlist { playlist } => serde_json::json!({
                "play_type": self.play_type().as_u8(),
                "playlist": playlist,
            }),
        }
    }
}

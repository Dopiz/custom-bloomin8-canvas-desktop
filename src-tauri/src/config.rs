//! Persisted app configuration (`device.json` in the Tauri app-data dir).
//!
//! The schema keeps a `devices` list (plus an `active_device_id`) so a
//! multi-device fleet is possible later (a non-goal for v1), even
//! though the UI only ever reads/writes the first entry.
//!
//! Kept free of any Tauri types so it can be unit tested against plain
//! `Path`s; `commands.rs` is the only place that resolves the real
//! `app_data_dir()`.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub const CONFIG_FILE_NAME: &str = "device.json";

/// One configured device. `id` is an opaque, UI-generated identifier (a
/// stable key even if `name`/`lan_ip` change later).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DeviceEntry {
    pub id: String,
    pub name: String,
    pub lan_ip: String,
    pub ble_name: String,
}

/// Top-level `device.json` contents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub devices: Vec<DeviceEntry>,
    #[serde(default)]
    pub active_device_id: Option<String>,
}

impl AppConfig {
    /// The device the UI/commands should act on: `active_device_id` if it
    /// resolves to a real entry, otherwise the first configured device.
    pub fn active_device(&self) -> Option<&DeviceEntry> {
        if let Some(id) = &self.active_device_id {
            if let Some(found) = self.devices.iter().find(|d| &d.id == id) {
                return Some(found);
            }
        }
        self.devices.first()
    }

    /// The device a schedule bound to `device_id` should push to. An empty
    /// `device_id` (a legacy schedule saved before per-device scheduling)
    /// falls back to [`active_device`], matching the frontend's migration
    /// behavior. A non-empty id that matches no configured device (the device
    /// was deleted) yields `None` — the scheduler treats that as a skip rather
    /// than retrying forever.
    pub fn device_for_schedule(&self, device_id: &str) -> Option<&DeviceEntry> {
        if device_id.trim().is_empty() {
            return self.active_device();
        }
        self.devices.iter().find(|d| d.id == device_id)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

/// Load `AppConfig` from `path`. A missing file is *not* an error — it's the
/// normal "first launch" case — and yields `AppConfig::default()`. A file
/// that exists but fails to parse (or fails to read for any other reason)
/// is reported as an error rather than silently discarded.
pub fn load(path: &Path) -> Result<AppConfig, ConfigError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AppConfig::default());
        }
        Err(source) => {
            return Err(ConfigError::Io {
                path: path.display().to_string(),
                source,
            });
        }
    };

    serde_json::from_str(&contents).map_err(|source| ConfigError::Parse {
        path: path.display().to_string(),
        source,
    })
}

/// Save `config` to `path`, creating parent directories as needed.
pub fn save(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }

    let json = serde_json::to_string_pretty(config).expect("AppConfig serialization is infallible");
    std::fs::write(path, json).map_err(|source| ConfigError::Io {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> AppConfig {
        AppConfig {
            devices: vec![DeviceEntry {
                id: "dev-1".to_string(),
                name: "Living Room".to_string(),
                lan_ip: "192.168.1.42".to_string(),
                ble_name: "Bloomin8-ABCD".to_string(),
            }],
            active_device_id: Some("dev-1".to_string()),
        }
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(CONFIG_FILE_NAME);
        let config = sample_config();

        save(&path, &config).expect("save should succeed");
        let loaded = load(&path).expect("load should succeed");

        assert_eq!(loaded, config);
    }

    #[test]
    fn missing_file_yields_default_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("does-not-exist.json");

        let loaded = load(&path).expect("missing file should not be an error");

        assert_eq!(loaded, AppConfig::default());
        assert!(loaded.devices.is_empty());
        assert!(loaded.active_device_id.is_none());
    }

    #[test]
    fn corrupt_json_is_reported_as_error_not_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(CONFIG_FILE_NAME);
        std::fs::write(&path, b"{ this is not json").expect("write corrupt file");

        let result = load(&path);

        assert!(matches!(result, Err(ConfigError::Parse { .. })));
    }

    #[test]
    fn active_device_falls_back_to_first_when_id_unset() {
        let mut config = sample_config();
        config.active_device_id = None;

        assert_eq!(config.active_device(), config.devices.first());
    }

    #[test]
    fn active_device_falls_back_to_first_when_id_unknown() {
        let mut config = sample_config();
        config.active_device_id = Some("does-not-exist".to_string());

        assert_eq!(config.active_device(), config.devices.first());
    }

    fn two_device_config() -> AppConfig {
        AppConfig {
            devices: vec![
                DeviceEntry {
                    id: "dev-1".to_string(),
                    name: "Living Room".to_string(),
                    lan_ip: "192.168.1.42".to_string(),
                    ble_name: "Bloomin8-AAAA".to_string(),
                },
                DeviceEntry {
                    id: "dev-2".to_string(),
                    name: "Bedroom".to_string(),
                    lan_ip: "192.168.1.99".to_string(),
                    ble_name: "Bloomin8-BBBB".to_string(),
                },
            ],
            active_device_id: Some("dev-1".to_string()),
        }
    }

    #[test]
    fn device_for_schedule_resolves_its_own_device_not_the_active_one() {
        let config = two_device_config();
        // A schedule bound to dev-2 resolves dev-2 even though dev-1 is active
        // — this is what makes a schedule push to its own device regardless of
        // the current UI selection (distinct lan_ip => distinct base URL).
        let dev2 = config.device_for_schedule("dev-2").expect("dev-2 resolves");
        assert_eq!(dev2.id, "dev-2");
        assert_eq!(dev2.lan_ip, "192.168.1.99");
        assert_ne!(dev2.lan_ip, config.active_device().unwrap().lan_ip);
    }

    #[test]
    fn device_for_schedule_empty_id_falls_back_to_active() {
        // Legacy schedules.json entries have no device_id (deserialize to "").
        let config = two_device_config();
        assert_eq!(config.device_for_schedule(""), config.active_device());
    }

    #[test]
    fn device_for_schedule_unknown_id_yields_none() {
        // The bound device was deleted from config — the scheduler treats this
        // as a skip rather than pushing to some other device.
        let config = two_device_config();
        assert_eq!(config.device_for_schedule("deleted-device"), None);
    }
}

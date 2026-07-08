//! Bloomin8 Canvas device protocol client.
//!
//! Ports the semantics of the reference implementation at
//! `~/.claude/skills/bloomin8-canvas/scripts/client.py` into
//! a typed async Rust client. Firmware gotchas (aggressive sleep, filename
//! caching, etc.) are encoded here so UI/scheduler code never has to think
//! about them directly.
//!
//! Wired into the running app via `commands.rs` (the Device
//! page). `mock` is additionally available outside tests behind the
//! `mock-device` feature, so `src-tauri/src/bin/mockdevice.rs` can stand up a
//! real MockDevice process for manual/E2E verification.
#![allow(dead_code)]

pub mod client;
pub mod error;
pub mod types;
pub mod wake;

pub use client::DeviceClient;
pub use error::DeviceError;
pub use types::{
    DeviceInfo, DeviceSettingsUpdate, DeviceState, GalleryImage, GallerySummary, PlayType,
    PlaylistSummary, ShowRequest, UploadAndShowResult,
};

#[cfg(any(test, feature = "mock-device"))]
pub mod mock;

#[cfg(any(test, feature = "mock-device"))]
pub use mock::MockDevice;

#[cfg(test)]
mod tests;

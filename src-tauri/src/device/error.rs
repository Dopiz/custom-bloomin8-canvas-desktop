use std::time::Duration;

use thiserror::Error;

use super::types::DeviceState;

#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("http request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("failed to parse device response: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("device did not reach Ready within {0:?} (last state: {1:?})")]
    NotReady(Duration, Option<DeviceState>),

    /// After a `show_now` upload, `deviceInfo.image` must end
    /// with the freshly-uploaded filename. A mismatch means the firmware is
    /// still serving a stale/cached image even though `/state` looked Ready.
    #[error("display verification failed: expected image ending in '{expected}', device reports {actual:?}")]
    DisplayVerificationFailed {
        expected: String,
        actual: Option<String>,
    },

    /// `cleanup` refuses to run when `keep` is empty, since a
    /// failed upload piped into cleanup would otherwise delete every
    /// same-prefix image, including the one currently displayed.
    #[error("cleanup refused: 'keep' filename is empty (did the preceding upload fail?)")]
    CleanupRefused,

    /// The BLE wake pulse and subsequent poll both failed to bring the
    /// device up within the allotted budget.
    #[error("device did not wake within {0:?}")]
    WakeTimeout(Duration),

    /// The device answered with a non-2xx status and a `{status:"fail",
    /// msg:"…"}` body — surface its message (e.g. `NAME_TOO_LONG`) rather than
    /// a bare HTTP 500.
    #[error("{0}")]
    Device(String),
}

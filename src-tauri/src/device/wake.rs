//! BLE wake pulse for the Bloomin8 Canvas.
//!
//! Ports `~/.claude/skills/bloomin8-canvas/wake.py`: discover the device by
//! advertised BLE name, write a wake pulse to a fixed GATT characteristic,
//! then disconnect.
//!
//! The pulse itself is fire-and-forget from the caller's
//! point of view — the BLE stack (here `btleplug`, `bleak` in the Python
//! reference) is known to sometimes error during the post-pulse disconnect
//! even though the pulse landed and the device is waking up. The *real*
//! success signal is HTTP: whoever calls [`ble_wake_pulse`] must poll
//! `GET /deviceInfo` afterwards and let that decide, never this function's
//! `Result`. Accordingly every BLE error in here is logged, not propagated.

use std::time::Duration;

use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Manager, Peripheral};
use uuid::Uuid;

/// GATT characteristic the wake pulse is written to.
pub const WAKE_CHAR_UUID: Uuid = Uuid::from_u128(0x0000f001_0000_1000_8000_00805f9b34fb);

/// Default advertised BLE name to scan for.
pub const DEFAULT_BLE_NAME: &str = "Bloomin8";

/// How long the pulse holds `0x01` before dropping back to `0x00`
/// (the HA reference component's 1ms gap is too short for firmware
/// 1.8.35 — 500ms wakes reliably, mirroring a real button hold).
const PULSE_GAP: Duration = Duration::from_millis(500);

/// How long to scan for peripherals before giving up.
const SCAN_TIMEOUT: Duration = Duration::from_secs(10);

/// One BLE peripheral matching a scan hint — returned to the frontend by the
/// `ble_scan` command so the add-device flow can confirm the device is in range.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BleMatch {
    /// The advertised local name (equals the device's own name, per the
    /// verified firmware behavior).
    pub name: String,
    /// Received signal strength (dBm; higher/closer to 0 is stronger).
    pub rssi: i16,
}

/// The effective match hint: the caller's substring, or [`DEFAULT_BLE_NAME`]
/// when they left it blank.
fn effective_hint(hint: &str) -> String {
    let trimmed = hint.trim();
    if trimmed.is_empty() {
        DEFAULT_BLE_NAME.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Scan for BLE peripherals whose advertised local name (case-insensitive)
/// *contains* `hint`, returning each one's [`Peripheral`] handle plus its
/// advertised name and RSSI. Best-effort: a missing adapter yields an empty
/// list rather than an error (callers decide success via `/deviceInfo`).
async fn scan_candidates(
    hint: &str,
    timeout: Duration,
) -> Result<Vec<(Peripheral, String, i16)>, btleplug::Error> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let Some(central) = adapters.into_iter().next() else {
        eprintln!("ble: no BLE adapter available");
        return Ok(Vec::new());
    };

    central.start_scan(ScanFilter::default()).await?;
    tokio::time::sleep(timeout).await;
    let peripherals = central.peripherals().await?;
    let _ = central.stop_scan().await;

    let hint_lc = hint.to_lowercase();
    let mut out = Vec::new();
    for p in peripherals {
        if let Ok(Some(props)) = p.properties().await {
            if let Some(name) = props.local_name {
                if name.to_lowercase().contains(&hint_lc) {
                    out.push((p, name, props.rssi.unwrap_or(i16::MIN)));
                }
            }
        }
    }
    Ok(out)
}

/// Scan for Bloomin8-like peripherals matching `hint` (empty -> "Bloomin8"),
/// returning matches sorted strongest-signal first. Best-effort: any BLE error
/// (no adapter, missing permission) is logged and yields an empty list.
pub async fn ble_scan(hint: &str) -> Vec<BleMatch> {
    let hint = effective_hint(hint);
    match scan_candidates(&hint, SCAN_TIMEOUT).await {
        Ok(candidates) => {
            let mut matches: Vec<BleMatch> = candidates
                .into_iter()
                .map(|(_, name, rssi)| BleMatch { name, rssi })
                .collect();
            matches.sort_by(|a, b| b.rssi.cmp(&a.rssi));
            matches
        }
        Err(e) => {
            eprintln!("ble scan: {e}");
            Vec::new()
        }
    }
}

/// Choose which candidate to wake from a non-empty list. Prefers a candidate
/// whose advertised name equals the stored real device `name`
/// (case-insensitive); otherwise the strongest RSSI. Returns the index into
/// `candidates`. Pure so it can be unit-tested without real BLE hardware.
fn pick_target(candidates: &[(String, i16)], name: Option<&str>) -> usize {
    if let Some(name) = name {
        let name_lc = name.to_lowercase();
        if let Some(i) = candidates.iter().position(|(n, _)| n.to_lowercase() == name_lc) {
            return i;
        }
    }
    candidates
        .iter()
        .enumerate()
        .max_by_key(|(_, (_, rssi))| *rssi)
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Send the BLE wake pulse to the single best-matching device.
///
/// `hint` is the user's BLE match substring (empty -> "Bloomin8"); `name` is
/// the device's stored real name, used to disambiguate when several
/// peripherals match the hint. Exactly one peripheral is ever woken — the one
/// whose name equals `name`, else the strongest signal. Returns that
/// peripheral's advertised name so the caller can persist it.
///
/// This is deliberately "best effort": every failure mode (adapter missing,
/// device not found, connect failure, characteristic missing, write failure,
/// disconnect failure) is logged via `eprintln!` and swallowed. There is
/// intentionally no `Result` to propagate — callers decide whether the
/// device woke up by polling `/deviceInfo`, not by inspecting this
/// function's outcome.
///
/// # Manual acceptance (no BLE hardware in CI)
/// 1. Put the real Canvas to sleep (idle past `max_idle`, or `POST /sleep`).
/// 2. Call `wake_if_needed()` against the real device's `DeviceClient`.
/// 3. Observe: BLE scan finds "Bloomin8", pulse is written, and within ~45s
///    `GET /deviceInfo` succeeds again — even if this function logs a BLE
///    error during disconnect (that's expected).
pub async fn ble_wake_pulse(hint: &str, name: Option<&str>) -> Option<String> {
    match ble_wake_pulse_inner(hint, name).await {
        Ok(matched) => matched,
        Err(e) => {
            eprintln!("ble wake pulse: non-fatal error, continuing (poll /deviceInfo decides success): {e}");
            None
        }
    }
}

async fn ble_wake_pulse_inner(
    hint: &str,
    name: Option<&str>,
) -> Result<Option<String>, btleplug::Error> {
    let hint = effective_hint(hint);
    let mut candidates = scan_candidates(&hint, SCAN_TIMEOUT).await?;
    if candidates.is_empty() {
        eprintln!("ble wake pulse: no device advertising a name containing '{hint}' found (out of range, advertising off, or missing Bluetooth permission)");
        return Ok(None);
    }

    // Pick exactly one target (name match wins, else strongest signal) — never
    // broadcast the pulse to every match.
    let view: Vec<(String, i16)> = candidates
        .iter()
        .map(|(_, n, r)| (n.clone(), *r))
        .collect();
    let idx = pick_target(&view, name);
    let (peripheral, matched_name, _rssi) = candidates.swap_remove(idx);

    peripheral.connect().await?;
    peripheral.discover_services().await?;

    let characteristic = peripheral
        .characteristics()
        .into_iter()
        .find(|c| c.uuid == WAKE_CHAR_UUID);

    if let Some(characteristic) = characteristic {
        peripheral
            .write(&characteristic, &[0x01], WriteType::WithoutResponse)
            .await?;
        tokio::time::sleep(PULSE_GAP).await;
        peripheral
            .write(&characteristic, &[0x00], WriteType::WithoutResponse)
            .await?;
    } else {
        eprintln!("ble wake pulse: characteristic {WAKE_CHAR_UUID} not found on device");
    }

    // Disconnect errors are exactly the ones called out as
    // non-fatal false negatives — the `?` above would already have
    // propagated write/connect errors, so anything from here on is just
    // logged by the caller via `ble_wake_pulse`.
    peripheral.disconnect().await?;
    Ok(Some(matched_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_char_uuid_matches_spec() {
        assert_eq!(
            WAKE_CHAR_UUID.to_string(),
            "0000f001-0000-1000-8000-00805f9b34fb"
        );
    }

    #[test]
    fn effective_hint_defaults_when_blank() {
        assert_eq!(effective_hint(""), "Bloomin8");
        assert_eq!(effective_hint("   "), "Bloomin8");
        assert_eq!(effective_hint(" Dopiz "), "Dopiz");
    }

    #[test]
    fn pick_target_prefers_exact_name_match_over_stronger_signal() {
        let candidates = vec![
            ("Other Bloomin8".to_string(), -40i16),
            ("Dopiz Bloomin8".to_string(), -70i16),
        ];
        // Weaker signal (-70) but exact name match wins.
        assert_eq!(pick_target(&candidates, Some("Dopiz Bloomin8")), 1);
        // Case-insensitive.
        assert_eq!(pick_target(&candidates, Some("dopiz bloomin8")), 1);
    }

    #[test]
    fn pick_target_falls_back_to_strongest_rssi() {
        let candidates = vec![
            ("Bloomin8 A".to_string(), -80i16),
            ("Bloomin8 B".to_string(), -35i16),
            ("Bloomin8 C".to_string(), -60i16),
        ];
        // No name provided -> strongest signal (-35, index 1).
        assert_eq!(pick_target(&candidates, None), 1);
        // Name provided but no candidate matches -> strongest signal.
        assert_eq!(pick_target(&candidates, Some("Not Present")), 1);
    }
}

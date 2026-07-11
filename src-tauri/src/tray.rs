//! Tray/menu-bar icon.
//!
//! - Menu: one "Refresh <name> now" per *enabled* schedule (read fresh from
//!   `schedules.json`), "Open app", and "Quit" — the only path that actually
//!   exits the process (closing the window must not quit).
//! - Tooltip reflects the last passively-observed device status — updated
//!   only when the UI already fetched `/deviceInfo` (via
//!   [`note_device_info`]); the tray never polls or wakes the device on its
//!   own. Combined with a failure badge set by
//!   [`crate::notify::handle_run_result`].
//!
//! Deliberately not wired up in `--capture-spike`/`--widgets-e2e`/
//! `--scheduler-e2e` mode — those modes never create a main window and must
//! keep exiting on their own (see `lib.rs`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

use crate::device::DeviceInfo;
use crate::scheduler::{self, SchedulerManager};

/// Fixed id so `lib.rs`/`commands.rs` can look the tray icon back up via
/// `AppHandle::tray_by_id` after creation (e.g. to rebuild the menu when
/// schedules change, or refresh the tooltip).
pub const TRAY_ID: &str = "main-tray";

/// Whether the most recently observed scheduled run failed and hasn't been
/// acknowledged yet. Cleared on the next successful run or when the user
/// opens the main window via the tray. A single flag is enough for v1 (one
/// device, no per-schedule badges).
#[derive(Default)]
pub struct FailureBadge(AtomicBool);

impl FailureBadge {
    fn set(&self, failed: bool) {
        self.0.store(failed, Ordering::SeqCst);
    }
    fn get(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Last passively-observed device-status text ("Online · 82%"), set only
/// when the UI already fetched `/deviceInfo` (see [`note_device_info`]) and
/// combined with [`FailureBadge`] to build the tray tooltip. Empty until the
/// first observation, which surfaces as "Status unknown".
#[derive(Default)]
struct DeviceStatus(Mutex<String>);

impl DeviceStatus {
    fn set(&self, s: String) {
        *self.0.lock().expect("DeviceStatus mutex poisoned") = s;
    }
    fn get(&self) -> String {
        self.0.lock().expect("DeviceStatus mutex poisoned").clone()
    }
}

/// Sets/clears the failure badge and refreshes the tray tooltip immediately.
/// Called by `notify::handle_run_result` after every completed scheduled
/// run, and by the tray's own "Open app" handler (opening the app
/// acknowledges the failure).
pub fn set_failure_badge(app: &AppHandle, failed: bool) {
    if let Some(badge) = app.try_state::<FailureBadge>() {
        badge.set(failed);
    }
    refresh_tooltip(app);
}

fn refresh_tooltip(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let status = app
        .try_state::<DeviceStatus>()
        .map(|s| s.get())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Status unknown".to_string());
    let failed = app.try_state::<FailureBadge>().map(|b| b.get()).unwrap_or(false);
    let prefix = if failed { "\u{26A0} " } else { "" };
    let _ = tray.set_tooltip(Some(format!("Bloomin8 \u{2014} {prefix}{status}")));
}

/// Passively records the latest device status for the tray tooltip. Called
/// from `commands::device_info` on a successful `/deviceInfo` fetch, so the
/// tooltip rides on info the UI already requested — the tray never polls or
/// wakes the device itself. A no-op before the tray is built (windowless
/// debug modes) since [`DeviceStatus`] is only managed in [`setup`].
pub fn note_device_info(app: &AppHandle, info: &DeviceInfo) {
    let status = match info.battery {
        Some(b) => format!("Online \u{b7} {b}%"),
        None => "Online".to_string(),
    };
    if let Some(state) = app.try_state::<DeviceStatus>() {
        state.set(status);
    }
    refresh_tooltip(app);
}

/// Builds the tray menu from whatever is on disk right now: one "Refresh
/// <name> now" entry per enabled schedule, then "Open app" and "Quit".
fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;

    let schedules = app
        .path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join(scheduler::SCHEDULES_FILE_NAME))
        .and_then(|path| scheduler::load_schedules(&path).ok())
        .unwrap_or_default();

    let mut any_schedule = false;
    for schedule in schedules.into_iter().filter(|s| s.enabled) {
        let item = MenuItem::with_id(
            app,
            format!("refresh:{}", schedule.id),
            format!("Refresh {} now", schedule.name),
            true,
            None::<&str>,
        )?;
        menu.append(&item)?;
        any_schedule = true;
    }
    if any_schedule {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }

    let open_item = MenuItem::with_id(app, "open-app", "Open app", true, None::<&str>)?;
    menu.append(&open_item)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    menu.append(&quit_item)?;

    Ok(menu)
}

/// Rebuilds the tray menu from `schedules.json` — call after any schedule
/// CRUD (`schedule_save`/`schedule_delete`) so the "Refresh <name> now"
/// entries stay in sync, mirroring how `SchedulerManager::reload` keeps the
/// live cron jobs in sync. A no-op if the tray hasn't been built yet (e.g.
/// windowless debug modes never call [`setup`]).
pub fn rebuild_menu(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    match build_menu(app) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => eprintln!("[tray] failed to rebuild menu: {e}"),
    }
}

fn open_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
    // Opening the app acknowledges any pending failure badge.
    set_failure_badge(app, false);
}

/// Builds the tray icon and wires menu-click handling. Call once at app
/// setup, only in the normal (windowed) run mode. The tooltip stays at its
/// initial "Status unknown" text until the UI first fetches `/deviceInfo`
/// (see [`note_device_info`]) — the tray never contacts the device itself.
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    app.manage(FailureBadge::default());
    app.manage(DeviceStatus::default());

    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("Bloomin8 \u{2014} Status unknown")
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let id = event.id().0.as_str();
            if id == "quit" {
                app.exit(0);
            } else if id == "open-app" {
                open_main_window(app);
            } else if let Some(schedule_id) = id.strip_prefix("refresh:") {
                let app = app.clone();
                let schedule_id = schedule_id.to_string();
                tauri::async_runtime::spawn(async move {
                    let Some(manager) = app.try_state::<Arc<SchedulerManager>>() else {
                        eprintln!("[tray] scheduler not ready yet, ignoring refresh click");
                        return;
                    };
                    if let Err(e) = manager.run_now(&app, &schedule_id).await {
                        eprintln!("[tray] refresh {schedule_id} failed: {e}");
                    }
                });
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;

    Ok(())
}

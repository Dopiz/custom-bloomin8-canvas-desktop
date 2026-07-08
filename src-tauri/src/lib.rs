mod capture;
mod commands;
mod config;
pub mod device;
mod fixtures;
mod last_push;
mod library;
mod notify;
mod render_service;
mod scheduler;
mod tray;
mod widgets;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// Output directory for `--capture-spike`: `spike-output/` in the repo.
fn spike_out_dir() -> PathBuf {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.join("../spike-output")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Debug tool: `--capture-spike` renders all 3 widgets x 2
    // orientations through the real pipeline with fixture data, windowless
    // (simulates the tray background scheduler), then exits.
    let args: Vec<String> = std::env::args().collect();
    let spike_mode = args.iter().any(|a| a == "--capture-spike");
    // Debug/E2E tool: `--widgets-e2e <mock-base-url>` runs
    // preview+push against a running MockDevice using the real render_service
    // pipeline (real Open-Meteo API for weather), windowless, then exits.
    let widgets_e2e_url = args
        .iter()
        .position(|a| a == "--widgets-e2e")
        .and_then(|i| args.get(i + 1))
        .cloned();
    // Debug/E2E tool: `--scheduler-e2e <mock-base-url>` runs
    // the scheduler's overlap-prevention/retry-once/history state machine
    // against a running MockDevice end-to-end, windowless, then exits — no
    // GUI session or real cron wait required.
    let scheduler_e2e_url = args
        .iter()
        .position(|a| a == "--scheduler-e2e")
        .and_then(|i| args.get(i + 1))
        .cloned();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        // "Launch at login" toggle on the Device page. Defaults to
        // disabled — this only registers the plugin (JS calls
        // `enable`/`disable`/`isEnabled`); it never turns autostart on by
        // itself. `MacosLauncher::LaunchAgent` avoids the AppleScript
        // System Events prompt.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(capture::CaptureState::default())
        .invoke_handler(tauri::generate_handler![
            greet,
            capture::get_capture_payload,
            capture::notify_capture_ready,
            capture::submit_capture_error,
            capture::spike_log,
            commands::get_config,
            commands::save_config,
            commands::device_info,
            commands::device_wake,
            commands::ble_scan,
            commands::device_sleep,
            commands::device_reboot,
            commands::device_clear_screen,
            commands::device_set_settings,
            commands::gallery_list,
            commands::gallery_create,
            commands::gallery_delete,
            commands::gallery_images,
            commands::image_delete,
            commands::show_image,
            commands::show_gallery,
            commands::show_playlist,
            commands::show_next,
            commands::playlist_list,
            commands::playlist_get,
            commands::playlist_save,
            commands::playlist_delete,
            commands::preview_widget,
            commands::push_widget,
            commands::fetch_image,
            commands::purge_device_data,
            commands::preview_image,
            commands::push_image,
            commands::ip_geolocation,
            commands::reverse_geocode,
            library::library_list,
            library::library_add,
            library::library_delete,
            library::library_image,
            commands::schedules_list,
            commands::schedule_save,
            commands::schedule_delete,
            commands::schedule_run_now,
            commands::history_list,
            last_push::last_push,
        ])
        .setup(move |app| {
            if spike_mode {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let out_dir = spike_out_dir();
                    let code = match capture::run_capture_spike(&handle, &out_dir).await {
                        Ok(()) => 0,
                        Err(e) => {
                            eprintln!("[capture-spike] FAILED: {e}");
                            1
                        }
                    };
                    handle.exit(code);
                });
            } else if let Some(mock_url) = widgets_e2e_url.clone() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let code = match render_service::run_widgets_e2e_check(&handle, &mock_url).await
                    {
                        Ok(()) => 0,
                        Err(e) => {
                            eprintln!("[widgets-e2e] FAILED: {e}");
                            1
                        }
                    };
                    handle.exit(code);
                });
            } else if let Some(mock_url) = scheduler_e2e_url.clone() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let code = match scheduler::run_scheduler_e2e_check(&handle, &mock_url).await {
                        Ok(()) => 0,
                        Err(e) => {
                            eprintln!("[scheduler-e2e] FAILED: {e}");
                            1
                        }
                    };
                    handle.exit(code);
                });
            } else {
                // Main window is created here (not in tauri.conf.json) so spike
                // mode can run windowless.
                let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("Bloomin8")
                    .inner_size(1120.0, 820.0)
                    .min_inner_size(900.0, 640.0)
                    .build()?;

                // Closing the window must not quit the app — the
                // scheduler keeps running in the tray. Only
                // the tray's own "Quit" item (`tray::setup`'s on_menu_event,
                // which calls `AppHandle::exit`) actually exits. This
                // `WindowEvent::CloseRequested` handler is only attached to
                // the real main window in this (non-spike/non-e2e) branch,
                // so the windowless CLI debug modes keep exiting normally.
                {
                    let window_for_hide = window.clone();
                    window.on_window_event(move |event| {
                        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                            api.prevent_close();
                            let _ = window_for_hide.hide();
                        }
                    });
                }

                // Start the background scheduler so cron-driven pushes
                // keep running once the window is up. Skipped in spike/e2e
                // debug modes (handled above) since those exit immediately.
                // `manage()` happens synchronously (blocking just on the
                // cheap `JobScheduler::new()`) *before* `setup()` returns, so
                // `tauri::State<Arc<SchedulerManager>>` is never unmanaged
                // when a frontend command runs; only the actual cron-loop
                // `start()` (which loads schedules.json and may run pushes)
                // is left running in the background.
                let app_data_dir = app
                    .path()
                    .app_data_dir()
                    .expect("app data dir must resolve");
                let schedules_path = app_data_dir.join(scheduler::SCHEDULES_FILE_NAME);
                let history_path = app_data_dir.join(scheduler::HISTORY_FILE_NAME);
                let manager = tauri::async_runtime::block_on(scheduler::SchedulerManager::new(
                    schedules_path,
                    history_path,
                    scheduler::DEFAULT_RETRY_DELAY,
                ))
                .expect("scheduler must initialize");
                let manager = Arc::new(manager);

                // Wire scheduled-run results (any status) to the
                // tray/notification layer. Purely observational — does not
                // touch `execute_run`'s retry/overlap decision logic.
                let notify_handle = app.handle().clone();
                tauri::async_runtime::block_on(
                    manager.set_on_result(move |entry| notify::handle_run_result(&notify_handle, &entry)),
                );

                app.manage(manager.clone());
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = manager.start(handle).await {
                        eprintln!("[scheduler] failed to start: {e}");
                    }
                });

                // Tray icon + menu + device-status poll. Only built in
                // this normal windowed run mode.
                tray::setup(&app.handle().clone())?;
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        // Tauri exits when the last window closes — and during background
        // (windowless) captures the *hidden capture window* is the last
        // window, so its teardown between two captures killed the app before
        // the second image was produced. A window-close exit
        // arrives as `ExitRequested { code: None }`; explicit `app.exit(code)`
        // carries `Some(code)` and must stay honored, so only the former is
        // prevented, and only while captures are in flight.
        if let RunEvent::ExitRequested { code: None, api, .. } = &event {
            if app_handle.state::<capture::CaptureState>().is_active() {
                api.prevent_exit();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    const CRYPTO_TEMPLATE: &str = include_str!("../templates/crypto-template.html");
    const WEATHER_TEMPLATE: &str = include_str!("../templates/weather-template.html");
    const COUNTDOWN_TEMPLATE: &str = include_str!("../templates/countdown-template.html");

    #[test]
    fn bundled_templates_are_readable_and_contain_placeholders() {
        for (name, contents) in [
            ("crypto-template.html", CRYPTO_TEMPLATE),
            ("weather-template.html", WEATHER_TEMPLATE),
            ("countdown-template.html", COUNTDOWN_TEMPLATE),
        ] {
            assert!(!contents.is_empty(), "{name} should not be empty");
            assert!(
                contents.contains("{{"),
                "{name} should contain template placeholders (\"{{{{\")"
            );
        }
    }
}

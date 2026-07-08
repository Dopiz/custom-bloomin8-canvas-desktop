//! OS notification on scheduled-run failure.
//!
//! Deliberately decoupled from `scheduler::execute_run` — this module only
//! reacts to the [`scheduler::HistoryEntry`] the scheduler's `on_result`
//! hook already hands it (see `scheduler::SchedulerManager::set_on_result`),
//! wired up once in `lib.rs` setup. It never touches retry/overlap
//! decision logic.

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::scheduler::{self, HistoryEntry, RunStatus};
use crate::tray;

/// Looks up a schedule's display name by id from `schedules.json`, falling
/// back to the raw id if the schedule was deleted (or the app-data dir
/// can't be resolved) between the run starting and this callback firing.
fn schedule_name(app: &AppHandle, schedule_id: &str) -> String {
    use tauri::Manager;
    let path = match app.path().app_data_dir() {
        Ok(dir) => dir.join(scheduler::SCHEDULES_FILE_NAME),
        Err(_) => return schedule_id.to_string(),
    };
    scheduler::load_schedules(&path)
        .ok()
        .and_then(|list| list.into_iter().find(|s| s.id == schedule_id))
        .map(|s| s.name)
        .unwrap_or_else(|| schedule_id.to_string())
}

/// Pure title/body formatting for a failed run's OS notification — kept
/// separate from `handle_run_result` so it's unit-testable without a
/// running Tauri app.
pub(crate) fn failure_notification_content(schedule_name: &str, entry: &HistoryEntry) -> (String, String) {
    let title = format!("Bloomin8: {schedule_name} failed");
    let body = entry
        .error
        .clone()
        .unwrap_or_else(|| "scheduled run failed (no error detail recorded)".to_string());
    (title, body)
}

/// [`scheduler::SchedulerManager::on_result`] callback body (installed in
/// `lib.rs` setup): updates the tray failure badge, and for `Failed` runs
/// also fires an OS notification via `tauri-plugin-notification`.
pub fn handle_run_result(app: &AppHandle, entry: &HistoryEntry) {
    match entry.status {
        RunStatus::Failed => {
            tray::set_failure_badge(app, true);
            let name = schedule_name(app, &entry.schedule_id);
            let (title, body) = failure_notification_content(&name, entry);
            if let Err(e) = app.notification().builder().title(title).body(body).show() {
                eprintln!("[notify] failed to show OS notification: {e}");
            }
        }
        RunStatus::Success => tray::set_failure_badge(app, false),
        RunStatus::Skipped => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_now() -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 7)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
    }

    #[test]
    fn failure_notification_content_includes_schedule_name_and_error() {
        let entry = HistoryEntry {
            schedule_id: "s1".to_string(),
            started_at: fixed_now(),
            finished_at: fixed_now(),
            status: RunStatus::Failed,
            error: Some("attempt 1 failed: timeout; retry failed: timeout".to_string()),
            filename: None,
        };

        let (title, body) = failure_notification_content("Weather every 30m", &entry);

        assert!(title.contains("Weather every 30m"), "title was {title:?}");
        assert!(title.to_lowercase().contains("failed"), "title was {title:?}");
        assert!(body.contains("timeout"), "body was {body:?}");
    }

    #[test]
    fn failure_notification_content_falls_back_when_error_missing() {
        let entry = HistoryEntry {
            schedule_id: "s1".to_string(),
            started_at: fixed_now(),
            finished_at: fixed_now(),
            status: RunStatus::Failed,
            error: None,
            filename: None,
        };

        let (_, body) = failure_notification_content("Crypto hourly", &entry);

        assert!(!body.is_empty());
    }
}

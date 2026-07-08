//! Scheduler — cron-driven recurring widget pushes.
//!
//! Split into two halves on purpose:
//! - The decision logic ([`execute_run`], overlap prevention, retry-once,
//!   history persistence, `schedules.json` round-trip) is plain async Rust
//!   with an injected push closure and clock, so it's unit-testable with no
//!   `tokio-cron-scheduler`, no real device, and no real render pipeline.
//! - [`SchedulerManager`] is the thin `tokio-cron-scheduler` wiring on top —
//!   it turns each enabled [`Schedule`]'s cron string into a job that calls
//!   [`execute_run`] with a real push closure built from
//!   `render_service::push_widget_config`.
//!
//! Reuses `render_service::push_widget_config` unmodified (the
//! wake_if_needed -> render -> upload_and_show orchestration) — this module
//! never touches rendering or device I/O directly.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler};
use uuid::Uuid;

use crate::commands;
use crate::device::DeviceClient;
use crate::render_service;
use crate::widgets::config::{PanelOrientation, WidgetConfig, WidgetRenderConfig};

pub const SCHEDULES_FILE_NAME: &str = "schedules.json";
pub const HISTORY_FILE_NAME: &str = "history.jsonl";

/// Retry once after 60s on failure — kept as a plain `Duration`
/// (not hardcoded inside [`execute_run`]) so tests can inject a millisecond
/// delay instead of waiting a full minute.
pub const DEFAULT_RETRY_DELAY: Duration = Duration::from_secs(60);

fn default_true() -> bool {
    true
}

/// One user-defined recurring push: a widget config plus a cron expression.
/// `id` is an opaque, UI-generated identifier (stable even if `name`/`cron`
/// change later), also used as the overlap-prevention and history key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Schedule {
    pub id: String,
    pub name: String,
    /// The device this schedule pushes to, identified by
    /// [`crate::config::DeviceEntry::id`]. `#[serde(default)]` (empty string)
    /// lets a legacy `schedules.json` written before per-device scheduling
    /// deserialize; an empty value is resolved to the active device at run
    /// time (see [`crate::commands::client_for_device`]).
    #[serde(default)]
    pub device_id: String,
    pub widget: WidgetRenderConfig,
    /// Standard 6-field (with seconds) or 5-field cron expression, as
    /// accepted by `tokio-cron-scheduler`/`croner`.
    pub cron: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SchedulesError {
    #[error("failed to read schedules file {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse schedules file {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

/// Load `schedules.json` from `path`. A missing file yields an empty list
/// (normal first-launch case, mirroring `config::load`); a file that exists
/// but fails to parse is a reported error, never a silent empty list.
pub fn load_schedules(path: &Path) -> Result<Vec<Schedule>, SchedulesError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(SchedulesError::Io {
                path: path.display().to_string(),
                source,
            })
        }
    };
    serde_json::from_str(&contents).map_err(|source| SchedulesError::Parse {
        path: path.display().to_string(),
        source,
    })
}

/// Save the full schedule list to `path`, creating parent directories as
/// needed.
pub fn save_schedules(path: &Path, schedules: &[Schedule]) -> Result<(), SchedulesError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| SchedulesError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let json = serde_json::to_string_pretty(schedules).expect("Schedule serialization is infallible");
    std::fs::write(path, json).map_err(|source| SchedulesError::Io {
        path: path.display().to_string(),
        source,
    })
}

// --- Run history -------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Success,
    Failed,
    Skipped,
}

/// One line of `history.jsonl` — one execution attempt of one schedule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    pub schedule_id: String,
    pub started_at: NaiveDateTime,
    pub finished_at: NaiveDateTime,
    pub status: RunStatus,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
}

/// Appends one entry to `history.jsonl` (append-only, one JSON object per
/// line), creating the file/parent directories on first write.
pub fn append_history(path: &Path, entry: &HistoryEntry) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    let line = serde_json::to_string(entry).expect("HistoryEntry serialization is infallible");
    writeln!(file, "{line}")
}

/// Reads up to `limit` most-recent history entries (newest first). A
/// missing file yields an empty list; malformed individual lines are
/// skipped rather than failing the whole read (append-only logs can end up
/// with a torn last line if the process was killed mid-write).
pub fn read_history(path: &Path, limit: usize) -> std::io::Result<Vec<HistoryEntry>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(source),
    };
    let mut entries: Vec<HistoryEntry> = contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    entries.reverse();
    entries.truncate(limit);
    Ok(entries)
}

// --- Overlap prevention + retry-once state machine ----------------------

/// Tracks which schedule ids currently have a run in flight (overlap
/// prevention — skip if the same schedule is still running).
#[derive(Clone, Default)]
pub struct RunningSet(Arc<Mutex<HashSet<String>>>);

impl RunningSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// True if `schedule_id` is currently marked as running (test/debug
    /// helper — production code goes through [`execute_run`]).
    #[allow(dead_code)]
    pub async fn contains(&self, schedule_id: &str) -> bool {
        self.0.lock().await.contains(schedule_id)
    }
}

/// Executes one scheduled run of `schedule_id` and appends exactly one
/// [`HistoryEntry`] to `history_path`:
/// - if another run of the same schedule is already in flight, records
///   `Skipped` and returns immediately without calling `push`.
/// - otherwise calls `push()`; on failure waits `retry_delay` and calls
///   `push()` once more; only a second failure is recorded as `Failed`.
///
/// `now` is invoked once per timestamp needed (not `chrono::Local::now()`
/// directly) so tests can inject a fixed/advancing clock.
pub async fn execute_run<F, Fut>(
    running: &RunningSet,
    schedule_id: &str,
    history_path: &Path,
    retry_delay: Duration,
    mut now: impl FnMut() -> NaiveDateTime,
    mut push: F,
) -> std::io::Result<HistoryEntry>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let started_at = now();

    {
        let mut guard = running.0.lock().await;
        if guard.contains(schedule_id) {
            let entry = HistoryEntry {
                schedule_id: schedule_id.to_string(),
                started_at,
                finished_at: started_at,
                status: RunStatus::Skipped,
                error: Some("a previous run of this schedule is still in flight".to_string()),
                filename: None,
            };
            append_history(history_path, &entry)?;
            return Ok(entry);
        }
        guard.insert(schedule_id.to_string());
    }

    let entry = match push().await {
        Ok(filename) => HistoryEntry {
            schedule_id: schedule_id.to_string(),
            started_at,
            finished_at: now(),
            status: RunStatus::Success,
            error: None,
            filename: Some(filename),
        },
        Err(first_err) => {
            tokio::time::sleep(retry_delay).await;
            match push().await {
                Ok(filename) => HistoryEntry {
                    schedule_id: schedule_id.to_string(),
                    started_at,
                    finished_at: now(),
                    status: RunStatus::Success,
                    error: None,
                    filename: Some(filename),
                },
                Err(second_err) => HistoryEntry {
                    schedule_id: schedule_id.to_string(),
                    started_at,
                    finished_at: now(),
                    status: RunStatus::Failed,
                    error: Some(format!(
                        "attempt 1 failed: {first_err}; retry failed: {second_err}"
                    )),
                    filename: None,
                },
            }
        }
    };

    running.0.lock().await.remove(schedule_id);
    append_history(history_path, &entry)?;
    Ok(entry)
}

// --- tokio-cron-scheduler wiring ------------------------------------------

/// Panel pixel size assumed when no device is reachable yet — mirrors
/// `commands::DEFAULT_PANEL_WIDTH/HEIGHT`. `client` is resolved from the
/// schedule's *own* `device_id` by the caller ([`run_scheduled`]), so a
/// schedule always pushes to its bound device regardless of which device the
/// UI currently has selected.
async fn push_for_schedule(
    app: &AppHandle,
    client: &DeviceClient,
    widget: &WidgetRenderConfig,
) -> Result<String, String> {
    let (panel_width, panel_height) = match client.info().await {
        Ok(info) => (info.width, info.height),
        Err(_) => (commands::DEFAULT_PANEL_WIDTH, commands::DEFAULT_PANEL_HEIGHT),
    };

    // Image schedules bypass the widget render/capture pipeline: read the
    // library original, size it to the panel with the manual push dialog's
    // exact settings, then push it the same way `commands::push_image` does.
    if let WidgetConfig::Image {
        library_id,
        mode,
        border,
    } = &widget.widget
    {
        // A deleted library image surfaces here as a clear error string, which
        // `execute_run` records as a failed run (no panic, no retry loop bug).
        let bytes = crate::library::read_image_bytes(app, library_id)?;
        let landscape = matches!(widget.orientation, PanelOrientation::Landscape);
        let jpeg = commands::process_push_image(
            &bytes,
            panel_width,
            panel_height,
            landscape,
            true, // rotate clockwise (image schedules don't expose direction)
            mode,
            border == "white",
        )?;
        client.wake_if_needed().await.map_err(|e| e.to_string())?;
        let now = chrono::Local::now().naive_local();
        let timestamp = now.format("%Y%m%d_%H%M%S").to_string();
        let outcome = client
            .upload_and_show(jpeg, widget.widget.upload_prefix(), commands::WIDGET_GALLERY, &timestamp)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(outcome.filename);
    }

    let cache_dir = commands::widget_cache_dir(app)?;
    let now = chrono::Local::now().naive_local();
    let outcome = render_service::push_widget_config(
        app,
        client,
        widget,
        panel_width,
        panel_height,
        &cache_dir,
        commands::WIDGET_GALLERY,
        now,
    )
    .await?;
    Ok(outcome.filename)
}

/// Resolves a schedule's own device client (from its `device_id`) and drives
/// it through [`execute_run`]. If the bound device can't be resolved — it was
/// deleted from `device.json` — this records a single `Skipped` history entry
/// for the tick instead of letting `execute_run`'s retry-once path fire twice
/// against a device that will never come back. `execute_run`'s
/// overlap/retry/history semantics are otherwise untouched: only the choice of
/// which device's client to push to moved here.
async fn run_scheduled(
    app: &AppHandle,
    running: &RunningSet,
    schedule_id: &str,
    device_id: &str,
    widget: &WidgetRenderConfig,
    history_path: &Path,
    retry_delay: Duration,
) -> std::io::Result<HistoryEntry> {
    let client = match commands::client_for_device(app, device_id) {
        Ok(client) => client,
        Err(err) => {
            let now = chrono::Local::now().naive_local();
            let entry = HistoryEntry {
                schedule_id: schedule_id.to_string(),
                started_at: now,
                finished_at: now,
                status: RunStatus::Skipped,
                error: Some(format!("run skipped: {err}")),
                filename: None,
            };
            append_history(history_path, &entry)?;
            return Ok(entry);
        }
    };

    let entry = execute_run(
        running,
        schedule_id,
        history_path,
        retry_delay,
        || chrono::Local::now().naive_local(),
        || {
            let app = app.clone();
            let client = client.clone();
            let widget = widget.clone();
            async move { push_for_schedule(&app, &client, &widget).await }
        },
    )
    .await?;

    // Remember this scheduled push (image *or* widget branch both surface their
    // filename via `entry.filename`) so the Device-page hero can show a
    // landscape image upright. Recorded under the schedule's *resolved* device
    // id (a legacy empty id maps to the active device) and only on success.
    // Best-effort: `record` swallows its own errors.
    if entry.status == RunStatus::Success {
        if let (Some(filename), Some(resolved_id)) = (
            entry.filename.as_deref(),
            commands::resolved_schedule_device_id(app, device_id),
        ) {
            crate::last_push::record(
                app,
                &resolved_id,
                filename,
                commands::orientation_str(widget.orientation),
            );
        }
    }

    Ok(entry)
}

/// Callback fired once per completed [`execute_run`] (any status —
/// `Success`/`Failed`/`Skipped`), used by the tray/notification wiring to
/// surface failures without `execute_run` itself knowing anything about tray
/// icons or OS notifications. Deliberately a thin `Fn`, not a trait, so tests
/// can inject a plain closure.
pub type OnRunResult = Arc<dyn Fn(HistoryEntry) + Send + Sync>;

/// Owns the live `tokio-cron-scheduler` instance plus the bookkeeping needed
/// to CRUD schedules at runtime: which cron-job uuid backs which schedule
/// id, the overlap-prevention set, and the on-disk paths. Held in Tauri
/// state as `Arc<SchedulerManager>`.
pub struct SchedulerManager {
    jobs: Mutex<JobScheduler>,
    job_ids: Mutex<HashMap<String, Uuid>>,
    running: RunningSet,
    schedules_path: PathBuf,
    history_path: PathBuf,
    retry_delay: Duration,
    /// Result hook: set once at app setup via [`SchedulerManager::set_on_result`].
    /// Wrapped in `Arc<Mutex<..>>` (not a plain field) because `add_job`'s
    /// cron closure is `'static` and needs its own clone of the slot — it
    /// can't borrow `self` — while the callback itself is installed *after*
    /// construction (it needs an `AppHandle` clone that isn't available
    /// inside `new`).
    on_result: Arc<Mutex<Option<OnRunResult>>>,
}

impl SchedulerManager {
    pub async fn new(
        schedules_path: PathBuf,
        history_path: PathBuf,
        retry_delay: Duration,
    ) -> Result<Self, String> {
        let jobs = JobScheduler::new().await.map_err(|e| e.to_string())?;
        Ok(Self {
            jobs: Mutex::new(jobs),
            job_ids: Mutex::new(HashMap::new()),
            running: RunningSet::new(),
            schedules_path,
            history_path,
            retry_delay,
            on_result: Arc::new(Mutex::new(None)),
        })
    }

    /// Installs (or replaces) the result callback. Does not touch
    /// `execute_run`'s decision logic — only observes the [`HistoryEntry`] it
    /// already produces, after the fact.
    pub async fn set_on_result(&self, cb: impl Fn(HistoryEntry) + Send + Sync + 'static) {
        *self.on_result.lock().await = Some(Arc::new(cb));
    }

    async fn notify_result(&self, entry: &HistoryEntry) {
        let cb = self.on_result.lock().await.clone();
        if let Some(cb) = cb {
            cb(entry.clone());
        }
    }

    /// Starts the underlying cron loop and loads `schedules.json`, wiring a
    /// job for each enabled schedule. Call once at app setup.
    pub async fn start(&self, app: AppHandle) -> Result<(), String> {
        self.reload(&app).await?;
        self.jobs.lock().await.start().await.map_err(|e| e.to_string())
    }

    /// Drops all currently-registered cron jobs and re-adds one per enabled
    /// schedule read fresh from `schedules.json`. Called after any
    /// CRUD mutation (`schedule_save`/`schedule_delete`) so the live cron
    /// set always matches disk.
    pub async fn reload(&self, app: &AppHandle) -> Result<(), String> {
        {
            let mut ids = self.job_ids.lock().await;
            let sched = self.jobs.lock().await;
            for (_, uuid) in ids.drain() {
                let _ = sched.remove(&uuid).await;
            }
        }

        let schedules = load_schedules(&self.schedules_path).map_err(|e| e.to_string())?;
        for schedule in schedules.into_iter().filter(|s| s.enabled) {
            self.add_job(app.clone(), schedule).await?;
        }
        Ok(())
    }

    async fn add_job(&self, app: AppHandle, schedule: Schedule) -> Result<(), String> {
        let running = self.running.clone();
        let history_path = self.history_path.clone();
        let retry_delay = self.retry_delay;
        let schedule_id = schedule.id.clone();
        let device_id = schedule.device_id.clone();
        let widget = schedule.widget.clone();
        let on_result = self.on_result.clone();

        let job = Job::new_async(schedule.cron.as_str(), move |_uuid, _lock| {
            let app = app.clone();
            let running = running.clone();
            let history_path = history_path.clone();
            let schedule_id = schedule_id.clone();
            let device_id = device_id.clone();
            let widget = widget.clone();
            let on_result = on_result.clone();
            Box::pin(async move {
                let result = run_scheduled(
                    &app,
                    &running,
                    &schedule_id,
                    &device_id,
                    &widget,
                    &history_path,
                    retry_delay,
                )
                .await;
                match result {
                    Ok(entry) => {
                        let cb = on_result.lock().await.clone();
                        if let Some(cb) = cb {
                            cb(entry);
                        }
                    }
                    Err(e) => {
                        eprintln!("[scheduler] failed to persist history for {schedule_id}: {e}");
                    }
                }
            })
        })
        .map_err(|e| e.to_string())?;

        let uuid = job.guid();
        self.jobs.lock().await.add(job).await.map_err(|e| e.to_string())?;
        self.job_ids.lock().await.insert(schedule.id, uuid);
        Ok(())
    }

    /// Runs `schedule_id` immediately (Tauri `schedule_run_now` command),
    /// going through the exact same [`execute_run`] decision logic (overlap
    /// prevention still applies against any in-flight cron-triggered run of
    /// the same id) and recording history the same way.
    pub async fn run_now(&self, app: &AppHandle, schedule_id: &str) -> Result<HistoryEntry, String> {
        let schedules = load_schedules(&self.schedules_path).map_err(|e| e.to_string())?;
        let schedule = schedules
            .into_iter()
            .find(|s| s.id == schedule_id)
            .ok_or_else(|| format!("no schedule with id {schedule_id:?}"))?;

        let entry = run_scheduled(
            app,
            &self.running,
            schedule_id,
            &schedule.device_id,
            &schedule.widget,
            &self.history_path,
            self.retry_delay,
        )
        .await
        .map_err(|e| e.to_string())?;
        self.notify_result(&entry).await;
        Ok(entry)
    }
}

// ---------------------------------------------------------------------------
// `--scheduler-e2e` debug mode: automated MockDevice E2E for the acceptance
// criterion, fully scriptable without a GUI session (unlike BLE/physical
// panel acceptance, which stays on the manual checklist). Exercises the real
// `execute_run` decision logic and the real render/push pipeline against a
// running MockDevice: success, overlap-skip, retry-then-fail (unreachable
// device), and recovery.
// ---------------------------------------------------------------------------

pub async fn run_scheduler_e2e_check(app: &AppHandle, mock_base_url: &str) -> Result<(), String> {
    eprintln!("[scheduler-e2e] starting against {mock_base_url}");
    use crate::device::DeviceClient;
    use crate::widgets::config::{PanelOrientation, RotateDirection, WidgetConfig};

    let cache_dir = std::env::temp_dir().join("bloomin8-scheduler-e2e-cache");
    let history_path = std::env::temp_dir().join(format!(
        "bloomin8-scheduler-e2e-history-{}.jsonl",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&history_path);

    let weather_req = WidgetRenderConfig {
        widget: WidgetConfig::Weather {
            lat: 25.033,
            lon: 121.565,
            city: "Taipei".to_string(),
            force_icon: None,
        },
        orientation: PanelOrientation::Portrait,
        rotate: RotateDirection::Cw,
    };

    let running = RunningSet::new();
    let schedule_id = "scheduler-e2e-weather";
    let short_retry = Duration::from_millis(200);

    // 1. First run against a reachable MockDevice must succeed and the
    // device must show a filename ending in what `execute_run` recorded.
    let good_client = DeviceClient::new(mock_base_url.to_string());
    {
        let app = app.clone();
        let req = weather_req.clone();
        let cache_dir = cache_dir.clone();
        let client = good_client.clone();
        let entry = execute_run(
            &running,
            schedule_id,
            &history_path,
            short_retry,
            || chrono::Local::now().naive_local(),
            || {
                let app = app.clone();
                let req = req.clone();
                let cache_dir = cache_dir.clone();
                let client = client.clone();
                async move {
                    render_service::push_widget_config(
                        &app,
                        &client,
                        &req,
                        1200,
                        1600,
                        &cache_dir,
                        "default",
                        chrono::Local::now().naive_local(),
                    )
                    .await
                    .map(|o| o.filename)
                }
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        if entry.status != RunStatus::Success {
            return Err(format!("expected first run to succeed, got {entry:?}"));
        }
        let info = good_client.info().await.map_err(|e| e.to_string())?;
        let shown = info.image.unwrap_or_default();
        let filename = entry.filename.clone().unwrap_or_default();
        if !shown.ends_with(&filename) {
            return Err(format!("deviceInfo.image {shown:?} does not end with {filename:?}"));
        }
        println!("[scheduler-e2e] run 1 OK: pushed {filename}, device shows {shown}");
    }

    // 2. Overlap prevention: mark the schedule as already running, then
    // execute_run must skip without calling push at all.
    {
        running.0.lock().await.insert(schedule_id.to_string());
        let entry = execute_run(
            &running,
            schedule_id,
            &history_path,
            short_retry,
            || chrono::Local::now().naive_local(),
            || async { Err::<String, String>("push should not be called while overlapping".to_string()) },
        )
        .await
        .map_err(|e| e.to_string())?;
        if entry.status != RunStatus::Skipped {
            return Err(format!("expected overlap-skip, got {entry:?}"));
        }
        running.0.lock().await.remove(schedule_id);
        println!("[scheduler-e2e] overlap prevention OK: run 2 skipped");
    }

    // 3. Retry-then-fail: point at an unreachable address so both attempts
    // fail, then confirm the history line records `Failed`.
    {
        let unreachable = DeviceClient::new("http://127.0.0.1:1".to_string());
        let app = app.clone();
        let req = weather_req.clone();
        let cache_dir = cache_dir.clone();
        let entry = execute_run(
            &running,
            schedule_id,
            &history_path,
            short_retry,
            || chrono::Local::now().naive_local(),
            || {
                let app = app.clone();
                let req = req.clone();
                let cache_dir = cache_dir.clone();
                let client = unreachable.clone();
                async move {
                    render_service::push_widget_config(
                        &app,
                        &client,
                        &req,
                        1200,
                        1600,
                        &cache_dir,
                        "default",
                        chrono::Local::now().naive_local(),
                    )
                    .await
                    .map(|o| o.filename)
                }
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        if entry.status != RunStatus::Failed {
            return Err(format!("expected retry-then-fail, got {entry:?}"));
        }
        println!("[scheduler-e2e] retry-then-fail OK: {:?}", entry.error);
    }

    // 4. Recovery: same schedule against the good MockDevice again succeeds.
    {
        let app = app.clone();
        let req = weather_req.clone();
        let cache_dir = cache_dir.clone();
        let client = good_client.clone();
        let entry = execute_run(
            &running,
            schedule_id,
            &history_path,
            short_retry,
            || chrono::Local::now().naive_local(),
            || {
                let app = app.clone();
                let req = req.clone();
                let cache_dir = cache_dir.clone();
                let client = client.clone();
                async move {
                    render_service::push_widget_config(
                        &app,
                        &client,
                        &req,
                        1200,
                        1600,
                        &cache_dir,
                        "default",
                        chrono::Local::now().naive_local(),
                    )
                    .await
                    .map(|o| o.filename)
                }
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        if entry.status != RunStatus::Success {
            return Err(format!("expected recovery run to succeed, got {entry:?}"));
        }
        println!("[scheduler-e2e] recovery OK: pushed {}", entry.filename.unwrap_or_default());
    }

    let history = read_history(&history_path, 10).map_err(|e| e.to_string())?;
    println!("[scheduler-e2e] history.jsonl has {} entries recorded at {}", history.len(), history_path.display());
    for entry in &history {
        println!(
            "  {} {:?} {:?} {:?}",
            entry.schedule_id, entry.status, entry.filename, entry.error
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("bloomin8-scheduler-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("history.jsonl")
    }

    fn sample_schedule() -> Schedule {
        Schedule {
            id: "s1".to_string(),
            name: "Weather every 30m".to_string(),
            device_id: "dev-1".to_string(),
            widget: WidgetRenderConfig {
                widget: crate::widgets::config::WidgetConfig::Weather {
                    lat: 25.0,
                    lon: 121.0,
                    city: "Taipei".to_string(),
                    force_icon: None,
                },
                orientation: crate::widgets::config::PanelOrientation::Portrait,
                rotate: crate::widgets::config::RotateDirection::Cw,
            },
            cron: "0 */30 * * * *".to_string(),
            enabled: true,
        }
    }

    fn fixed_now() -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 7)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
    }

    // --- schedules.json round-trip ---

    #[test]
    fn schedules_round_trip_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SCHEDULES_FILE_NAME);
        let schedules = vec![sample_schedule()];

        save_schedules(&path, &schedules).unwrap();
        let loaded = load_schedules(&path).unwrap();

        assert_eq!(loaded, schedules);
    }

    #[test]
    fn missing_schedules_file_yields_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");

        let loaded = load_schedules(&path).unwrap();

        assert!(loaded.is_empty());
    }

    #[test]
    fn corrupt_schedules_json_is_reported_as_error_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SCHEDULES_FILE_NAME);
        std::fs::write(&path, b"{ not json").unwrap();

        let result = load_schedules(&path);

        assert!(matches!(result, Err(SchedulesError::Parse { .. })));
    }

    #[test]
    fn enabled_defaults_to_true_when_absent_from_json() {
        let json = serde_json::json!([{
            "id": "s1",
            "name": "n",
            "widget": {
                "kind": "weather", "lat": 0.0, "lon": 0.0, "city": "",
                "orientation": "portrait"
            },
            "cron": "* * * * * *"
        }]);
        let loaded: Vec<Schedule> = serde_json::from_value(json).unwrap();
        assert!(loaded[0].enabled);
    }

    #[test]
    fn device_id_defaults_to_empty_for_legacy_schedules_and_falls_back_to_active() {
        // A schedules.json written before per-device scheduling has no
        // `device_id`. It must still deserialize (empty string), and an empty
        // device_id resolves to the active device at run time — the migration
        // contract with `AppConfig::device_for_schedule` (unit-tested in
        // config.rs). Together they guarantee legacy schedules keep working.
        let json = serde_json::json!([{
            "id": "s1",
            "name": "n",
            "widget": {
                "kind": "weather", "lat": 0.0, "lon": 0.0, "city": "",
                "orientation": "portrait"
            },
            "cron": "* * * * * *",
            "enabled": true
        }]);
        let loaded: Vec<Schedule> = serde_json::from_value(json).unwrap();
        assert_eq!(loaded[0].device_id, "");

        let config = crate::config::AppConfig {
            devices: vec![crate::config::DeviceEntry {
                id: "dev-1".to_string(),
                name: "Living Room".to_string(),
                lan_ip: "192.168.1.42".to_string(),
                ble_name: String::new(),
            }],
            active_device_id: Some("dev-1".to_string()),
        };
        assert_eq!(
            config.device_for_schedule(&loaded[0].device_id),
            config.active_device()
        );
    }

    // --- history.jsonl ---

    #[test]
    fn history_appends_and_reads_back_newest_first() {
        let path = tmp_path("history-order");
        for i in 0..3u32 {
            let entry = HistoryEntry {
                schedule_id: "s1".to_string(),
                started_at: fixed_now(),
                finished_at: fixed_now(),
                status: RunStatus::Success,
                error: None,
                filename: Some(format!("run-{i}.jpg")),
            };
            append_history(&path, &entry).unwrap();
        }

        let history = read_history(&path, 10).unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].filename.as_deref(), Some("run-2.jpg"));
        assert_eq!(history[2].filename.as_deref(), Some("run-0.jpg"));
    }

    #[test]
    fn history_read_respects_limit() {
        let path = tmp_path("history-limit");
        for i in 0..5u32 {
            let entry = HistoryEntry {
                schedule_id: "s1".to_string(),
                started_at: fixed_now(),
                finished_at: fixed_now(),
                status: RunStatus::Success,
                error: None,
                filename: Some(format!("run-{i}.jpg")),
            };
            append_history(&path, &entry).unwrap();
        }

        let history = read_history(&path, 2).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].filename.as_deref(), Some("run-4.jpg"));
    }

    #[test]
    fn missing_history_file_yields_empty_list() {
        let path = tmp_path("history-missing").join("nested").join("gone.jsonl");
        let history = read_history(&path, 10).unwrap();
        assert!(history.is_empty());
    }

    #[test]
    fn malformed_history_lines_are_skipped_not_fatal() {
        let path = tmp_path("history-malformed");
        std::fs::write(&path, "not json at all\n").unwrap();
        let good = HistoryEntry {
            schedule_id: "s1".to_string(),
            started_at: fixed_now(),
            finished_at: fixed_now(),
            status: RunStatus::Success,
            error: None,
            filename: Some("ok.jpg".to_string()),
        };
        append_history(&path, &good).unwrap();

        let history = read_history(&path, 10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].filename.as_deref(), Some("ok.jpg"));
    }

    // --- execute_run: overlap prevention + retry-once state machine ---

    #[tokio::test]
    async fn execute_run_records_success_on_first_try() {
        let path = tmp_path("run-success");
        let running = RunningSet::new();

        let entry = execute_run(
            &running,
            "s1",
            &path,
            Duration::from_millis(1),
            || fixed_now(),
            || async { Ok("file.jpg".to_string()) },
        )
        .await
        .unwrap();

        assert_eq!(entry.status, RunStatus::Success);
        assert_eq!(entry.filename.as_deref(), Some("file.jpg"));
        assert!(!running.contains("s1").await);
    }

    #[tokio::test]
    async fn execute_run_retries_once_and_succeeds() {
        let path = tmp_path("run-retry-success");
        let running = RunningSet::new();
        let attempts = Cell::new(0u32);

        let entry = execute_run(
            &running,
            "s1",
            &path,
            Duration::from_millis(1),
            || fixed_now(),
            || {
                attempts.set(attempts.get() + 1);
                let n = attempts.get();
                async move {
                    if n == 1 {
                        Err("first attempt network error".to_string())
                    } else {
                        Ok("file.jpg".to_string())
                    }
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(attempts.get(), 2);
        assert_eq!(entry.status, RunStatus::Success);
        assert_eq!(entry.filename.as_deref(), Some("file.jpg"));
    }

    #[tokio::test]
    async fn execute_run_fails_after_two_attempts() {
        let path = tmp_path("run-fail-twice");
        let running = RunningSet::new();
        let attempts = Cell::new(0u32);

        let entry = execute_run(
            &running,
            "s1",
            &path,
            Duration::from_millis(1),
            || fixed_now(),
            || {
                attempts.set(attempts.get() + 1);
                async move { Err::<String, String>("still down".to_string()) }
            },
        )
        .await
        .unwrap();

        assert_eq!(attempts.get(), 2);
        assert_eq!(entry.status, RunStatus::Failed);
        assert!(entry.error.as_deref().unwrap().contains("still down"));
        assert!(!running.contains("s1").await);
    }

    #[tokio::test]
    async fn execute_run_skips_when_already_running_and_does_not_call_push() {
        let path = tmp_path("run-overlap");
        let running = RunningSet::new();
        running.0.lock().await.insert("s1".to_string());
        let called = Cell::new(false);

        let entry = execute_run(
            &running,
            "s1",
            &path,
            Duration::from_millis(1),
            || fixed_now(),
            || {
                called.set(true);
                async { Ok::<String, String>("should not happen".to_string()) }
            },
        )
        .await
        .unwrap();

        assert_eq!(entry.status, RunStatus::Skipped);
        assert!(!called.get(), "push must not be called while overlapping");
        // A skip doesn't touch the pre-existing running marker for the
        // in-flight run it deferred to.
        assert!(running.contains("s1").await);
    }

    #[tokio::test]
    async fn execute_run_appends_one_history_line_per_call() {
        let path = tmp_path("run-history-count");
        let running = RunningSet::new();

        for _ in 0..3 {
            execute_run(
                &running,
                "s1",
                &path,
                Duration::from_millis(1),
                || fixed_now(),
                || async { Ok("f.jpg".to_string()) },
            )
            .await
            .unwrap();
        }

        let history = read_history(&path, 10).unwrap();
        assert_eq!(history.len(), 3);
        assert!(history.iter().all(|e| e.schedule_id == "s1"));
    }

    // --- on_result hook (tray/notification wiring) ---

    /// The scheduler's result callback
    /// hook fires with the exact `HistoryEntry` produced for a failed run —
    /// this is what `notify::handle_run_result` hangs the "OS
    /// notification on failure" behavior off of. Exercised directly against
    /// `SchedulerManager::{set_on_result, notify_result}` rather than a full
    /// `run_now`/cron round trip, since a full round trip needs a live
    /// `AppHandle` (GUI-session-only); the decision logic itself
    /// (`execute_run`) is already covered by the tests above and is
    /// untouched by this hook.
    #[tokio::test]
    async fn on_result_hook_fires_with_failed_entry() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SchedulerManager::new(
            dir.path().join(SCHEDULES_FILE_NAME),
            dir.path().join(HISTORY_FILE_NAME),
            Duration::from_millis(1),
        )
        .await
        .unwrap();

        let captured: Arc<std::sync::Mutex<Option<HistoryEntry>>> = Arc::new(std::sync::Mutex::new(None));
        let captured_clone = captured.clone();
        manager
            .set_on_result(move |entry| {
                *captured_clone.lock().unwrap() = Some(entry);
            })
            .await;

        let failed_entry = HistoryEntry {
            schedule_id: "s1".to_string(),
            started_at: fixed_now(),
            finished_at: fixed_now(),
            status: RunStatus::Failed,
            error: Some("attempt 1 failed: network error; retry failed: network error".to_string()),
            filename: None,
        };
        manager.notify_result(&failed_entry).await;

        let got = captured.lock().unwrap().clone();
        assert_eq!(got, Some(failed_entry));
    }

    #[tokio::test]
    async fn on_result_hook_is_a_noop_when_unset() {
        // Guards against a panic/deadlock if notify_result is called before
        // set_on_result — the normal case for Success/Skipped runs before a
        // callback is ever installed (e.g. `--scheduler-e2e` mode).
        let dir = tempfile::tempdir().unwrap();
        let manager = SchedulerManager::new(
            dir.path().join(SCHEDULES_FILE_NAME),
            dir.path().join(HISTORY_FILE_NAME),
            Duration::from_millis(1),
        )
        .await
        .unwrap();

        let entry = HistoryEntry {
            schedule_id: "s1".to_string(),
            started_at: fixed_now(),
            finished_at: fixed_now(),
            status: RunStatus::Success,
            error: None,
            filename: Some("f.jpg".to_string()),
        };
        manager.notify_result(&entry).await;
    }
}

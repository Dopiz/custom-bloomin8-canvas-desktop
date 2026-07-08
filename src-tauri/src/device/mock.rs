//! In-process HTTP mock of a Bloomin8 Canvas device, for tests only.
//!
//! Reproduces the endpoints `DeviceClient` talks to, including firmware
//! quirks:
//!
//! - **Filename-cache bug**: once a filename has been uploaded, the *content*
//!   served for that filename is pinned to whatever was uploaded the first
//!   time. Deleting the file and re-uploading the same name does not refresh
//!   the cached content (`/state` still reports Ready and `deviceInfo.image`
//!   still points at the filename — only the actual displayed bytes are
//!   stale). Encoded here via `content_cache`, which `post_image_delete`
//!   deliberately never clears.
//! - **Sleep**: while asleep, every request hangs forever instead of
//!   responding (approximating "the device drops off the network"), so a
//!   caller with a client-side timeout observes a timeout error, same as
//!   against a real sleeping device.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::{Multipart, Query, Request, State},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::oneshot;

type SharedState = Arc<Mutex<MockState>>;

#[derive(Debug)]
struct MockState {
    width: u32,
    height: u32,
    battery: i64,
    max_idle: u64,
    sleep_duration: u64,
    idx_wake_sens: u64,
    name: String,
    gallery: String,
    current_image: Option<String>,
    status: i64,
    msg: String,
    asleep: bool,
    /// Filenames considered "present" in a gallery right now, in upload
    /// order (affects listing/delete only — NOT the cached content, see
    /// module docs).
    gallery_files: Vec<(String, String)>,
    /// (gallery, filename) -> content from the FIRST-EVER upload of that
    /// name. Intentionally never updated or cleared on delete.
    content_cache: HashMap<(String, String), Vec<u8>>,
    /// Test-only hook (see [`MockDevice::suppress_next_image_update`]): when
    /// true, the *next* `show_now` upload stores the file but leaves
    /// `current_image`/`gallery` untouched, simulating a device that never
    /// actually refreshed its displayed image.
    suppress_next_image_update: bool,
    /// Galleries that exist (independent of whether they contain images).
    galleries: Vec<String>,
    /// name -> raw JSON body, as last set via `PUT /playlist`.
    playlists: HashMap<String, Value>,
    /// What `/show` (or a `show_now` upload) last put on screen, exposed via
    /// `deviceInfo.now_playing` so tests can observe `show()`'s effect
    /// without a dedicated endpoint, e.g. `"image:/gallerys/default/f.jpg"`,
    /// `"gallery:vacation"`, `"playlist:morning"`.
    now_playing: Option<String>,
    hits: HashMap<&'static str, u32>,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 1600,
            battery: 80,
            max_idle: 120,
            sleep_duration: 86_400,
            idx_wake_sens: 0,
            name: "Bloomin8".to_string(),
            gallery: "default".to_string(),
            current_image: None,
            status: 100,
            msg: "Ready".to_string(),
            asleep: false,
            gallery_files: Vec::new(),
            content_cache: HashMap::new(),
            suppress_next_image_update: false,
            galleries: vec!["default".to_string()],
            playlists: HashMap::new(),
            now_playing: None,
            hits: HashMap::new(),
        }
    }
}

fn bump(state: &SharedState, key: &'static str) {
    let mut s = state.lock().unwrap();
    *s.hits.entry(key).or_insert(0) += 1;
}

/// A running mock device bound to an ephemeral localhost port. Dropping it
/// shuts down the background HTTP server task.
pub struct MockDevice {
    addr: SocketAddr,
    state: SharedState,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl MockDevice {
    /// Bind to an ephemeral localhost port (the usual case for tests, which
    /// only ever read the port back via [`Self::base_url`]).
    pub async fn start() -> Self {
        Self::start_at("127.0.0.1:0").await
    }

    /// Bind to a specific address, e.g. `"127.0.0.1:18080"` — used by the
    /// standalone `mockdevice` bin (feature `mock-device`) so a human can
    /// point the UI at a fixed, predictable port.
    pub async fn start_at(addr: &str) -> Self {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .unwrap_or_else(|e| panic!("bind mock device listener on {addr}: {e}"));
        let addr = listener.local_addr().expect("mock device local addr");

        let state: SharedState = Arc::new(Mutex::new(MockState::default()));
        let app = router(state.clone());

        let (tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    rx.await.ok();
                })
                .await
                .ok();
        });

        Self {
            addr,
            state,
            shutdown_tx: Some(tx),
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn set_asleep(&self, asleep: bool) {
        self.state.lock().unwrap().asleep = asleep;
    }

    pub fn is_asleep(&self) -> bool {
        self.state.lock().unwrap().asleep
    }

    pub fn set_status(&self, status: i64, msg: &str) {
        let mut s = self.state.lock().unwrap();
        s.status = status;
        s.msg = msg.to_string();
    }

    /// Flip to the given status/msg after `delay`, from a background task —
    /// used to simulate a device that becomes Ready some time after a
    /// `wait_ready` poll loop starts.
    pub fn set_status_after(&self, delay: Duration, status: i64, msg: &'static str) {
        let state = self.state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let mut s = state.lock().unwrap();
            s.status = status;
            s.msg = msg.to_string();
        });
    }

    pub fn hit_count(&self, endpoint: &str) -> u32 {
        *self.state.lock().unwrap().hits.get(endpoint).unwrap_or(&0)
    }

    pub fn stored_content(&self, gallery: &str, filename: &str) -> Option<Vec<u8>> {
        self.state
            .lock()
            .unwrap()
            .content_cache
            .get(&(gallery.to_string(), filename.to_string()))
            .cloned()
    }

    /// Test-only hook: make the *next* `show_now` upload behave as if the
    /// firmware silently failed to refresh the displayed image (bytes/name
    /// are still recorded as "present" in the gallery, but `current_image`
    /// is left as whatever it was before). Used to exercise
    /// `DeviceClient::upload_and_show`'s display-verification failure path.
    pub fn suppress_next_image_update(&self) {
        self.state.lock().unwrap().suppress_next_image_update = true;
    }

    /// Current list of `(gallery, filename)` pairs present in the mock, in
    /// upload order — handy for asserting on cleanup behaviour in tests.
    pub fn gallery_file_names(&self, gallery: &str) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .gallery_files
            .iter()
            .filter(|(g, _)| g == gallery)
            .map(|(_, name)| name.clone())
            .collect()
    }
}

impl Drop for MockDevice {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

fn router(state: SharedState) -> Router {
    Router::new()
        .route("/deviceInfo", get(get_device_info))
        .route("/state", get(get_state))
        .route("/whistle", get(get_whistle))
        .route("/sleep", post(post_sleep))
        .route("/reboot", post(post_reboot))
        .route("/clearScreen", post(post_clear_screen))
        .route("/settings", post(post_settings))
        .route("/upload", post(post_upload))
        .route("/image/delete", post(post_image_delete))
        .route(
            "/gallery",
            get(get_gallery).put(put_gallery_create).delete(delete_gallery),
        )
        .route("/gallery/list", get(get_gallery_list))
        .route("/show", post(post_show))
        .route("/showNext", post(post_show_next))
        .route(
            "/playlist",
            get(get_playlist).put(put_playlist).delete(delete_playlist),
        )
        .route("/playlist/list", get(get_playlist_list))
        .layer(middleware::from_fn_with_state(state.clone(), sleep_gate))
        .with_state(state)
}

/// While the mock is "asleep", every request just hangs — never responding —
/// approximating a device that has dropped off the LAN. This is enough for a
/// client with a request timeout to observe a timeout error, exactly like
/// against real sleeping hardware.
async fn sleep_gate(State(state): State<SharedState>, req: Request, next: Next) -> Response {
    let asleep = state.lock().unwrap().asleep;
    if asleep {
        std::future::pending::<()>().await;
    }
    next.run(req).await
}

async fn get_device_info(State(state): State<SharedState>) -> Json<Value> {
    bump(&state, "deviceInfo");
    let s = state.lock().unwrap();
    Json(json!({
        "width": s.width,
        "height": s.height,
        "battery": s.battery,
        "image": s.current_image,
        "gallery": s.gallery,
        "max_idle": s.max_idle,
        "sleep_duration": s.sleep_duration,
        "idx_wake_sens": s.idx_wake_sens,
        "name": s.name,
        "now_playing": s.now_playing,
    }))
}

async fn get_state(State(state): State<SharedState>) -> Json<Value> {
    bump(&state, "state");
    let s = state.lock().unwrap();
    Json(json!({ "status": s.status, "msg": s.msg }))
}

async fn get_whistle(State(state): State<SharedState>) -> Json<Value> {
    bump(&state, "whistle");
    Json(json!({ "ok": true }))
}

async fn post_sleep(State(state): State<SharedState>) -> Json<Value> {
    bump(&state, "sleep");
    state.lock().unwrap().asleep = true;
    Json(json!({ "ok": true }))
}

async fn post_reboot(State(state): State<SharedState>) -> Json<Value> {
    bump(&state, "reboot");
    Json(json!({ "ok": true }))
}

async fn post_clear_screen(State(state): State<SharedState>) -> Json<Value> {
    bump(&state, "clearScreen");
    let mut s = state.lock().unwrap();
    s.current_image = None;
    s.status = 100;
    s.msg = "Ready".to_string();
    Json(json!({ "ok": true }))
}

#[derive(Debug, Deserialize)]
struct SettingsBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    sleep_duration: Option<u64>,
    #[serde(default)]
    max_idle: Option<u64>,
    #[serde(default)]
    idx_wake_sens: Option<u64>,
}

async fn post_settings(State(state): State<SharedState>, Json(body): Json<SettingsBody>) -> Json<Value> {
    bump(&state, "settings");
    let mut s = state.lock().unwrap();
    if let Some(v) = body.name {
        s.name = v;
    }
    if let Some(v) = body.sleep_duration {
        s.sleep_duration = v;
    }
    if let Some(v) = body.max_idle {
        s.max_idle = v;
    }
    if let Some(v) = body.idx_wake_sens {
        s.idx_wake_sens = v;
    }
    Json(json!({ "ok": true }))
}

fn default_gallery() -> String {
    "default".to_string()
}

#[derive(Debug, Deserialize)]
struct UploadQuery {
    filename: String,
    #[serde(default = "default_gallery")]
    gallery: String,
    #[serde(default)]
    show_now: Option<i32>,
}

async fn post_upload(
    State(state): State<SharedState>,
    Query(q): Query<UploadQuery>,
    mut multipart: Multipart,
) -> Json<Value> {
    bump(&state, "upload");

    let mut bytes: Vec<u8> = Vec::new();
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() == Some("image") {
            bytes = field.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
        }
    }

    let key = (q.gallery.clone(), q.filename.clone());
    let mut s = state.lock().unwrap();

    // Firmware bug: content is cached by filename forever —
    // the first upload of a given name wins, later uploads under the same
    // name are silently ignored content-wise.
    s.content_cache.entry(key.clone()).or_insert(bytes);
    if !s.gallery_files.contains(&key) {
        s.gallery_files.push(key);
    }

    if q.show_now.unwrap_or(1) != 0 {
        if s.suppress_next_image_update {
            // Test-only hook: pretend the firmware never actually updated
            // the displayed image for this upload.
            s.suppress_next_image_update = false;
        } else {
            s.current_image = Some(format!("/gallerys/{}/{}", q.gallery, q.filename));
            s.gallery = q.gallery.clone();
        }
        s.status = 100;
        s.msg = "Ready".to_string();
    }

    Json(json!({ "ok": true }))
}

#[derive(Debug, Deserialize)]
struct ImageDeleteQuery {
    image: String,
    #[serde(default = "default_gallery")]
    gallery: String,
}

async fn post_image_delete(State(state): State<SharedState>, Query(q): Query<ImageDeleteQuery>) -> Json<Value> {
    bump(&state, "imageDelete");
    let key = (q.gallery, q.image);
    // Only removes the file from the "existing" list. `content_cache` is
    // deliberately left untouched — see module docs for why.
    state.lock().unwrap().gallery_files.retain(|k| k != &key);
    Json(json!({ "ok": true }))
}

#[derive(Debug, Deserialize)]
struct GalleryQuery {
    gallery_name: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

async fn get_gallery(State(state): State<SharedState>, Query(q): Query<GalleryQuery>) -> Json<Value> {
    bump(&state, "gallery");
    let s = state.lock().unwrap();
    let names: Vec<&str> = s
        .gallery_files
        .iter()
        .filter(|(g, _)| g == &q.gallery_name)
        .map(|(_, name)| name.as_str())
        .collect();
    let total = names.len();
    let page: Vec<Value> = names
        .into_iter()
        .skip(q.offset)
        .take(q.limit)
        .map(|name| json!({ "name": name }))
        .collect();
    Json(json!({ "data": page, "total": total }))
}

async fn post_show(State(state): State<SharedState>, Json(body): Json<Value>) -> Json<Value> {
    bump(&state, "show");
    let mut s = state.lock().unwrap();
    s.status = 100;
    s.msg = "Ready".to_string();

    match body.get("play_type").and_then(Value::as_i64) {
        Some(0) => {
            let image = body.get("image").and_then(Value::as_str).unwrap_or_default();
            s.current_image = Some(image.to_string());
            s.now_playing = Some(format!("image:{image}"));
        }
        Some(1) => {
            let gallery = body.get("gallery").and_then(Value::as_str).unwrap_or_default();
            s.gallery = gallery.to_string();
            s.current_image = None;
            s.now_playing = Some(format!("gallery:{gallery}"));
        }
        Some(2) => {
            let playlist = body.get("playlist").and_then(Value::as_str).unwrap_or_default();
            s.current_image = None;
            s.now_playing = Some(format!("playlist:{playlist}"));
        }
        _ => {}
    }

    Json(json!({ "ok": true }))
}

async fn post_show_next(State(state): State<SharedState>) -> Json<Value> {
    bump(&state, "showNext");
    Json(json!({ "ok": true }))
}

#[derive(Debug, Deserialize)]
struct NameQuery {
    name: String,
}

async fn get_gallery_list(State(state): State<SharedState>) -> Json<Value> {
    bump(&state, "galleryList");
    let s = state.lock().unwrap();
    // Firmware 1.8.35 returns a bare top-level array here (verified against a
    // real device), NOT a `{"data": [...]}` envelope like paginated `/gallery`.
    let data: Vec<Value> = s.galleries.iter().map(|name| json!({ "name": name })).collect();
    Json(json!(data))
}

async fn put_gallery_create(State(state): State<SharedState>, Query(q): Query<NameQuery>) -> Json<Value> {
    bump(&state, "galleryCreate");
    let mut s = state.lock().unwrap();
    if !s.galleries.contains(&q.name) {
        s.galleries.push(q.name);
    }
    Json(json!({ "ok": true }))
}

async fn delete_gallery(State(state): State<SharedState>, Query(q): Query<NameQuery>) -> Json<Value> {
    bump(&state, "galleryDelete");
    let mut s = state.lock().unwrap();
    s.galleries.retain(|g| g != &q.name);
    s.gallery_files.retain(|(g, _)| g != &q.name);
    Json(json!({ "ok": true }))
}

async fn get_playlist_list(State(state): State<SharedState>) -> Json<Value> {
    bump(&state, "playlistList");
    let s = state.lock().unwrap();
    // Bare top-level array, matching the real device (see get_gallery_list).
    let data: Vec<Value> = s.playlists.keys().map(|name| json!({ "name": name })).collect();
    Json(json!(data))
}

async fn get_playlist(State(state): State<SharedState>, Query(q): Query<NameQuery>) -> Json<Value> {
    bump(&state, "playlistGet");
    let s = state.lock().unwrap();
    Json(s.playlists.get(&q.name).cloned().unwrap_or_else(|| json!({})))
}

async fn put_playlist(State(state): State<SharedState>, Json(body): Json<Value>) -> Json<Value> {
    bump(&state, "playlistPut");
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    state.lock().unwrap().playlists.insert(name, body);
    Json(json!({ "ok": true }))
}

async fn delete_playlist(State(state): State<SharedState>, Query(q): Query<NameQuery>) -> Json<Value> {
    bump(&state, "playlistDelete");
    state.lock().unwrap().playlists.remove(&q.name);
    Json(json!({ "ok": true }))
}

# Bloomin8 Desktop

A Tauri v2 desktop app that controls a Bloomin8 e-ink Canvas directly over the
LAN (no cloud, no auth): push images, manage galleries/playlists, render
widgets (crypto prices, weather, countdown), and cron-schedule recurring
refreshes. Scheduling is done entirely by this app's Rust side
(`tokio-cron-scheduler`), not a device-side API.

## Architecture

- **Frontend**: React 19 + TypeScript + Tailwind v4, in `src/`. Reusable
  primitives live in `src/components/ui/index.tsx` (`Card`, `Button`,
  `IconButton`, `PillTabs`, `Field`, `Input`, `Select`, `Toggle`, `StatusDot`,
  `Badge`, `ListRow`, `EmptyState`, `Spinner`, `Toast`) — use these, don't
  re-roll. Visual spec: `design-system/MASTER.md` (semantic color tokens,
  never hardcode colors).
- **Backend**: Rust, in `src-tauri/src/`. Key modules:
  - `device/` — `DeviceClient` (thin async HTTP client for the Canvas LAN
    protocol, ported from `~/.claude/skills/bloomin8-canvas/scripts/client.py`)
    plus `MockDevice` (`device/mock.rs`) for tests/dev. `wake.rs` handles the
    BLE wake pulse.
  - `widgets/` — one data fetcher per widget (`crypto.rs`, `weather.rs`,
    `countdown.rs`) plus `template.rs` for HTML template rendering.
  - `capture.rs` — renders a widget's HTML in a hidden `WebviewWindow` and
    uses WKWebView's native `takeSnapshot` to screenshot it to JPEG. Honors a
    `rotate_to_panel` flag (landscape widgets are captured at swapped
    dimensions, then rotated back to the panel's native portrait pixels).
  - `render_service.rs` — orchestrates one widget render/push end-to-end
    (fetch data -> render HTML -> capture -> `upload_and_show`). Shared
    unmodified by both `commands.rs` (manual push from the UI) and
    `scheduler.rs` (cron-driven push).
  - `scheduler.rs` — cron scheduling via `tokio-cron-scheduler`. Split into
    plain-async decision logic (overlap prevention, retry-once, history) that
    is unit-testable without a real scheduler or device, and thin
    `SchedulerManager` wiring on top.
  - `commands.rs` — Tauri `#[tauri::command]` entry points invoked from the
    frontend.
  - `config.rs` — reads/writes `device.json` (device base URL, etc.) in the
    app data dir.
  - `tray.rs` / `notify.rs` — tray icon/menu and OS notifications for
    scheduled-run results; app keeps running in the tray after the window is
    closed (only the tray's Quit item actually exits).

## Device protocol gotchas

- **Filenames must never be reused** — the firmware caches a rendered image
  by filename, so re-uploading under the same name can show a stale image.
  Every push uploads under a fresh `<prefix>_<timestamp>.jpg` name.
- After upload, verify success by checking `deviceInfo`'s currently-shown
  image actually ends with the new filename — a mismatch is a real error, not
  a soft failure.
- The device sleeps aggressively; a sleeping/unreachable device needs a BLE
  wake pulse (`device/wake.rs`) before HTTP calls will succeed again — poll
  `GET /deviceInfo` for up to ~45s after waking.
- Captured JPEGs must be exactly `panel_width x panel_height` pixels (native
  portrait panel size); landscape widgets get rotated back to that size.
- `GET /gallerys/list` and `GET /playlists/list`-style endpoints return a
  **top-level JSON array**, not `{ data: [...] }`.
- A specific gallery image's bytes are fetched via
  `GET /gallerys/<gallery>/<name>`.

## Dev commands

- `pnpm tauri dev` — run the app in dev mode.
- `pnpm build` — frontend build (`tsc && vite build`).
- `cargo test --manifest-path src-tauri/Cargo.toml` — Rust unit tests
  (exercised against `MockDevice`, no real hardware needed).
- `cargo run --features mock-device --bin mockdevice -- --port 18080` — start
  a standalone MockDevice for manual/e2e testing.
- Debug/E2E CLI flags on the main binary (windowless, exit with a status
  code): `--capture-spike` (render all widgets x orientations through the
  real pipeline with fixture data), `--widgets-e2e <mock-base-url>`
  (preview+push against a running MockDevice), `--scheduler-e2e
  <mock-base-url>` (drive the scheduler's overlap/retry/history state
  machine against a running MockDevice).

## Testing

- Rust unit tests target `MockDevice`, not the real Canvas.
- Screenshot/GUI-dependent tests (`capture.rs`, `--capture-spike`) need a real
  WindowServer session (macOS) — they won't work headless.

## Conventions

- Conventional commits (`feat:`, `fix:`, `chore:`, `docs:`).
- Use semantic Tailwind tokens from `design-system/MASTER.md`
  (`bg-surface`, `text-fg`, `text-accent`, etc.) — never hardcode colors.
- Inject time/randomness (clock, filenames) as parameters rather than reading
  them directly, so render/push/scheduler logic stays unit-testable.

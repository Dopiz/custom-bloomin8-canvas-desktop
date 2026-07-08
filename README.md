# Bloomin8 Desktop

[![Buy me a boba 🧋](https://img.shields.io/badge/Buy%20me%20a%20boba-%F0%9F%A7%8B-ff69b4?style=for-the-badge)](https://dopiz.bobaboba.me/)

A macOS / Windows desktop app that controls a **Bloomin8 colour e-ink Canvas**
directly over your LAN — **no cloud, no account, no sign-in**. Push photos and
widgets, manage the device's gallery and playlists, and schedule recurring
refreshes, all from a native app that lives in your tray.

Built with **Tauri v2 (Rust) + React 19 + TypeScript + Tailwind v4**.

## Features

- **Multiple devices** — add Canvases by LAN IP and switch between them from the
  sidebar. Every device keeps its own settings, gallery, and schedules; the
  active device's real name and Wi-Fi/IP are shown in the header.
- **Local library → framed push** — keep your own image originals on your
  machine, then push any of them with per-push display settings (orientation,
  fit / fill / auto, black or white border). Images are processed **client-side**
  to the exact panel size, with a live picture-frame preview.
- **On-device view** — browse the images actually stored on the Canvas.
- **Widgets** — crypto prices, weather (with one-tap "use my location"), and a
  countdown. Preview, then push, in portrait or landscape.
- **Per-device schedules** — cron-based recurring pushes of a widget *or* a
  fixed image from your library; enable/disable each schedule and review run
  history.
- **Device control** — rename, sleep timers, BLE wake, reboot, clear screen,
  and launch-at-login.
- **Light / dark theme.**

> **Scheduling is local.** The Canvas firmware has no scheduling API — the
> official phone app does it via its cloud. This app schedules entirely on your
> machine, so recurring pushes only fire while it is running (it stays in the
> tray after you close the window).

> **Playlists are disabled in this first release.** The device-native playlist
> flow (build a rotation from images already on the device) still needs work, so
> its UI is turned off for now — the code is kept behind a flag
> (`PLAYLISTS_ENABLED` in `src/components/OnDeviceDialog.tsx`) and will return in
> a later version.

## Open API reference

This app talks to the Canvas over its **LAN HTTP protocol** — the same
real-time, action-oriented endpoints the device exposes for `show` / `upload`
/ settings / gallery / playlist operations. There is **no scheduling endpoint
on the device**; all recurring pushes are driven locally by this app (see the
scheduling note above).

The device protocol is documented in BLOOMIN8's official Home Assistant
integration:

- Official OpenAPI spec: <https://github.com/ARPOBOT-BLOOMIN8/eink_canvas_home_assistant_component/blob/main/openapi.yaml>
- Home Assistant component repo: <https://github.com/ARPOBOT-BLOOMIN8/eink_canvas_home_assistant_component>

## Development

```sh
pnpm install
pnpm tauri dev          # run the app in dev mode
pnpm build              # frontend typecheck + build (tsc && vite build)
cargo test --manifest-path src-tauri/Cargo.toml   # Rust unit tests (MockDevice, no hardware)
```

## Releases (GitHub Actions)

A single, **manual** workflow — [`.github/workflows/release.yml`](.github/workflows/release.yml).
It never runs on push or PR; you trigger it yourself:

**Actions → Release → Run workflow**, enter a version (e.g. `0.0.1`).

It then, in one go:

1. **Bumps the version** everywhere — `package.json`,
   `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` (+ `Cargo.lock`) — then
   commits and tags it `vX.Y.Z`.
2. **Builds** the app for **macOS** (universal `.dmg` / `.app`) and **Windows**
   (`.msi` / NSIS `-setup.exe`).
3. **Publishes a GitHub Release** for that tag with all the installers attached,
   ready to download. (Uncheck "Publish now" to leave it as a draft first.)

### Signing macOS builds in CI (optional)

By default the CI build is **unsigned** (users right-click → Open on first
launch). To have the release automatically **signed + notarized** so it opens
with no Gatekeeper warning, add these repo secrets — the workflow already wires
them into the build, and skips signing when they're absent:

| Secret | How to get it |
| --- | --- |
| `APPLE_CERTIFICATE` | Export your **Developer ID Application** cert from Keychain as a `.p12`, then base64 it: `base64 -i cert.p12 \| pbcopy`. Requires an [Apple Developer](https://developer.apple.com/) account ($99/yr). |
| `APPLE_CERTIFICATE_PASSWORD` | The password you set when exporting the `.p12`. |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Your Name (TEAMID)` (see `security find-identity -v -p codesigning`). |
| `APPLE_ID` | Your Apple account email. |
| `APPLE_PASSWORD` | An **app-specific password** from [appleid.apple.com](https://appleid.apple.com) (not your login password). |
| `APPLE_TEAM_ID` | Your 10-character Team ID. |

Add them under **Settings → Secrets and variables → Actions → New repository
secret**, then run the Release workflow again. (Windows signing needs a separate
code-signing certificate and isn't wired up yet.)

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Build / Release (macOS)

`src-tauri/tauri.conf.json` sets `bundle.macOS.signingIdentity` to `null`, so
`pnpm tauri build` always produces a working, **unsigned** (ad-hoc signed)
`.app` and `.dmg` under `src-tauri/target/release/bundle/` — no Apple
Developer account required to build and smoke-test locally:

```sh
pnpm tauri build
# -> src-tauri/target/release/bundle/macos/bloomin8-desktop.app
# -> src-tauri/target/release/bundle/dmg/bloomin8-desktop_<version>_aarch64.dmg
```

An unsigned build runs fine on the machine that built it, but macOS
Gatekeeper will block it (or show the "unidentified developer" prompt) once
it's downloaded on another Mac, and it cannot ship through any distribution
channel that checks notarization. `Info.plist` (bundled from
`src-tauri/Info.plist`, merged automatically because it sits next to
`tauri.conf.json`) already declares `NSBluetoothAlwaysUsageDescription` /
`NSBluetoothPeripheralUsageDescription` for the BLE wake feature, and
`src-tauri/Entitlements.plist` has the hardened-runtime entitlements
(`com.apple.security.network.client`, `com.apple.security.device.bluetooth`)
needed once you sign for real.

### Signing + notarizing (requires an Apple Developer ID)

1. Set a signing identity — either export it as an env var Tauri reads
   automatically, or set `bundle.macOS.signingIdentity` in
   `tauri.conf.json`:

   ```sh
   export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
   ```

2. Build (this signs the `.app` with the hardened runtime + the
   entitlements in `src-tauri/Entitlements.plist` since `signingIdentity`
   is no longer `null`):

   ```sh
   pnpm tauri build
   ```

   Or sign an already-built bundle by hand:

   ```sh
   codesign --deep --force --options runtime \
     --entitlements src-tauri/Entitlements.plist \
     --sign "Developer ID Application: Your Name (TEAMID)" \
     src-tauri/target/release/bundle/macos/bloomin8-desktop.app
   ```

3. Notarize with `notarytool` (needs an app-specific password or API key —
   see `xcrun notarytool store-credentials`):

   ```sh
   ditto -c -k --keepParent \
     src-tauri/target/release/bundle/macos/bloomin8-desktop.app \
     /tmp/bloomin8-desktop.zip

   xcrun notarytool submit /tmp/bloomin8-desktop.zip \
     --keychain-profile "AC_PASSWORD" \
     --wait
   ```

4. Staple the ticket so the app verifies offline (e.g. right after
   download, before first launch):

   ```sh
   xcrun stapler staple src-tauri/target/release/bundle/macos/bloomin8-desktop.app
   # for the DMG too, if distributing that:
   xcrun stapler staple src-tauri/target/release/bundle/dmg/bloomin8-desktop_*.dmg
   ```

5. Verify:

   ```sh
   codesign --verify --deep --strict --verbose=2 src-tauri/target/release/bundle/macos/bloomin8-desktop.app
   spctl --assess --type execute --verbose src-tauri/target/release/bundle/macos/bloomin8-desktop.app
   ```

## Disclaimer

This is an **unofficial, third-party community tool**. It is **not affiliated
with, endorsed by, or supported by** ARPOBOT or BLOOMIN8. "Bloomin8" and any
related names, logos, or trademarks belong to their respective owners and are
used here **only for compatibility and identification purposes**.

The software is provided **"AS IS", without warranty of any kind**, express or
implied. **Use it at your own risk** — the author accepts no responsibility for
any impact on your device, data, or network.

It communicates directly with the Canvas over your local network using a
**device protocol that is not guaranteed to be stable or publicly supported**;
a firmware update may change or break it at any time.

---

Made with 🧋 by me boba — <https://dopiz.bobaboba.me/>

import { useEffect, useRef, useState } from "react";
import {
  disable as disableAutostart,
  enable as enableAutostart,
  isEnabled as isAutostartEnabled,
} from "@tauri-apps/plugin-autostart";
import {
  BatteryMedium,
  Eraser,
  Globe,
  Image as ImageIcon,
  Maximize,
  Moon,
  Power,
  RefreshCw,
  RotateCcw,
  Wifi,
} from "lucide-react";
import {
  deviceClearScreen,
  deviceReboot,
  deviceSetSettings,
  deviceSleep,
  deviceWake,
  errorMessage,
  fetchDeviceInfo,
  getConfig,
  lastPush,
} from "../api/device";
import type { AppConfig, DeviceEntry, DeviceInfo, PanelOrientation } from "../types";
import ConfirmDialog from "./ConfirmDialog";
import {
  Button,
  Card,
  DeviceImage,
  Field,
  Input,
  SectionHeader,
  Spinner,
  StatusDot,
  Toggle,
  cx,
  useToast,
} from "./ui";

type InfoState = "loading" | "loaded" | "error" | "empty";
type Action = "wake" | "sleep" | "reboot" | "clear" | "settings" | null;

/** Fixed hero height in px — must match the `h-[360px]` class on the hero card;
 * used to size a rotated landscape image so it covers the hero after transform. */
const HERO_HEIGHT = 360;

/** The active device entry (`active_device_id` if it resolves, else first). */
function activeDeviceOf(cfg: AppConfig | null): DeviceEntry | undefined {
  if (!cfg) return undefined;
  return (
    (cfg.active_device_id && cfg.devices.find((d) => d.id === cfg.active_device_id)) ||
    cfg.devices[0] ||
    undefined
  );
}

export default function DevicePage({
  onDeviceName,
}: {
  /** Reports the device's live settings name up to the shell so the sidebar
   * switcher stays in sync (e.g. after a rename). */
  onDeviceName?: (name: string) => void;
} = {}) {
  const toast = useToast();
  const [config, setConfig] = useState<AppConfig | null>(null);

  const [info, setInfo] = useState<DeviceInfo | null>(null);
  const [infoState, setInfoState] = useState<InfoState>("empty");
  const [infoError, setInfoError] = useState("");

  // Last-pushed record for the active device — lets the hero show a landscape
  // image (stored rotated 90° into the portrait panel file) the right way up.
  const [lastPushed, setLastPushed] = useState<{
    filename: string;
    orientation: PanelOrientation;
  } | null>(null);
  // The hero's live pixel width, measured so the rotated (landscape) image can
  // be sized to cover the fixed-height hero after a 90° transform.
  const heroRef = useRef<HTMLDivElement>(null);
  const [heroWidth, setHeroWidth] = useState(0);

  const [action, setAction] = useState<Action>(null);
  const [confirming, setConfirming] = useState<"reboot" | "clear" | null>(null);

  const [settingsName, setSettingsName] = useState("");
  const [settingsSleepDuration, setSettingsSleepDuration] = useState("");
  const [settingsMaxIdle, setSettingsMaxIdle] = useState("");
  const [settingsWakeSens, setSettingsWakeSens] = useState("");

  const hasDevice = Boolean(config?.devices.length);

  // "Launch at login" toggle, backed by tauri-plugin-autostart.
  // Defaults to off (whatever the OS reports on first read) — nothing here
  // enables it automatically.
  const [autostart, setAutostart] = useState(false);
  const [autostartBusy, setAutostartBusy] = useState(false);

  useEffect(() => {
    isAutostartEnabled()
      .then(setAutostart)
      .catch((e) => toast.show("error", errorMessage(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function toggleAutostart(next: boolean) {
    setAutostartBusy(true);
    try {
      if (next) {
        await enableAutostart();
      } else {
        await disableAutostart();
      }
      setAutostart(next);
    } catch (e) {
      toast.show("error", errorMessage(e));
    } finally {
      setAutostartBusy(false);
    }
  }

  // Track the hero's width so a rotated landscape image can be sized to cover
  // it (its post-rotation footprint is heroWidth × hero height).
  useEffect(() => {
    const el = heroRef.current;
    if (!el) return;
    const update = () => setHeroWidth(el.clientWidth);
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    getConfig()
      .then((cfg) => {
        setConfig(cfg);
        const active = activeDeviceOf(cfg);
        if (active?.lan_ip) {
          refreshInfo();
        } else {
          setInfoState("empty");
        }
      })
      .catch((e) => {
        setInfoState("error");
        setInfoError(errorMessage(e));
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function refreshInfo() {
    setInfoState("loading");
    setInfoError("");
    try {
      const result = await fetchDeviceInfo();
      setInfo(result);
      setInfoState("loaded");
      // Refresh the last-pushed record alongside device info so the hero knows
      // whether the currently-shown image should be rendered upright.
      lastPush()
        .then(setLastPushed)
        .catch(() => setLastPushed(null));
      onDeviceName?.(typeof result.name === "string" ? result.name : "");
      setSettingsName((result.name as string) ?? "");
      setSettingsSleepDuration(
        result.sleep_duration !== undefined ? String(result.sleep_duration) : "",
      );
      setSettingsMaxIdle(result.max_idle !== undefined ? String(result.max_idle) : "");
      setSettingsWakeSens(
        result.idx_wake_sens !== undefined ? String(result.idx_wake_sens) : "",
      );
    } catch (e) {
      setInfoState("error");
      setInfoError(errorMessage(e));
    }
  }

  async function runAction(
    kind: Exclude<Action, null | "settings">,
    fn: () => Promise<void>,
    successMessage: string,
  ) {
    setAction(kind);
    try {
      await fn();
      await refreshInfo();
      toast.show("success", successMessage);
    } catch (e) {
      toast.show("error", errorMessage(e));
    } finally {
      setAction(null);
    }
  }

  async function saveSettings(e: React.FormEvent) {
    e.preventDefault();
    setAction("settings");
    try {
      await deviceSetSettings({
        name: settingsName.trim() || undefined,
        sleep_duration: settingsSleepDuration.trim()
          ? Number(settingsSleepDuration)
          : undefined,
        max_idle: settingsMaxIdle.trim() ? Number(settingsMaxIdle) : undefined,
        idx_wake_sens: settingsWakeSens.trim() ? Number(settingsWakeSens) : undefined,
      });
      toast.show("success", "Settings saved");
      await refreshInfo();
    } catch (e) {
      toast.show("error", errorMessage(e));
    } finally {
      setAction(null);
    }
  }

  const activeDevice = activeDeviceOf(config);
  // Show the device's own settings name (from /deviceInfo) first; fall back to
  // the in-app nickname only when the device is unreachable.
  const deviceName = (info?.name as string) || activeDevice?.name || "Bloomin8";
  const wifiSsid = typeof info?.sta_ssid === "string" ? info.sta_ssid : "";
  const lanIp =
    (typeof info?.sta_ip === "string" && info.sta_ip) || activeDevice?.lan_ip || "";
  const statusTone: "online" | "offline" | "idle" =
    infoState === "loaded" ? "online" : infoState === "error" ? "offline" : "idle";
  const statusLabel =
    infoState === "loaded"
      ? "Online"
      : infoState === "error"
        ? "Unreachable"
        : infoState === "loading"
          ? "Connecting…"
          : "Not set up";

  // Basename of the currently-shown image (info.image is `/gallerys/<g>/<name>`).
  const heroImageName = info?.image ? (info.image.split("/").pop() ?? "") : "";
  // Guard: only render upright when the shown image is *exactly* the one we last
  // pushed as landscape. Any mismatch (a portrait push, or the image was changed
  // by another source — gallery, playlist, another device) keeps object-cover.
  const showUpright =
    Boolean(info?.image) &&
    lastPushed?.orientation === "landscape" &&
    heroImageName === lastPushed.filename;

  return (
    <div className="mx-auto max-w-3xl space-y-5 p-6">
      <h1 className="text-3xl font-extrabold tracking-tight text-fg">Device</h1>

      {/* Hero device card: current panel image fills the card as a background,
          with device name/status/meta/actions floating over a bottom gradient. */}
      <div
        ref={heroRef}
        className="relative h-[360px] overflow-hidden rounded-3xl bg-inverse text-inverse-fg shadow-lg"
      >
        {info?.image ? (
          <>
            {showUpright ? (
              // Landscape push: the panel file is the upright content rotated
              // 90° clockwise into the portrait panel. Undo it with a -90°
              // (counter-clockwise) transform. The pre-rotation box is sized
              // hero-height wide × hero-width tall, so after the swap its
              // footprint is exactly hero-width × hero-height; object-cover then
              // fills it with no distortion or letterboxing.
              <div
                className="absolute left-1/2 top-1/2"
                style={{
                  width: HERO_HEIGHT,
                  height: heroWidth || undefined,
                  transform: "translate(-50%, -50%) rotate(-90deg)",
                }}
              >
                <DeviceImage
                  gallery={info.gallery ?? "default"}
                  name={heroImageName}
                  className="h-full w-full bg-transparent"
                  imgClassName="object-cover object-center"
                  alt="Current panel image (landscape)"
                />
              </div>
            ) : (
              // Wrap in an absolute box — DeviceImage is `relative` internally
              // (for its own overlays), so we can't pass `absolute` to it.
              <div className="absolute inset-0">
                <DeviceImage
                  gallery={info.gallery ?? "default"}
                  name={heroImageName}
                  className="h-full w-full bg-transparent"
                  imgClassName="object-cover object-center"
                  alt="Current panel image"
                />
              </div>
            )}
            <div className="pointer-events-none absolute inset-x-0 bottom-0 h-2/3 bg-gradient-to-t from-black/90 via-black/55 to-transparent" />
          </>
        ) : (
          <div className="absolute inset-0 flex items-center justify-center bg-black">
            {infoState === "loading" ? (
              <Spinner size={24} />
            ) : (
              <ImageIcon size={28} className="text-white/30" aria-hidden />
            )}
          </div>
        )}

        <div className="relative z-10 flex h-full flex-col justify-end p-5">
          <span className="mb-1.5 flex items-center gap-1.5 text-sm font-semibold text-white/80">
            <StatusDot tone={statusTone} />
            {statusLabel}
          </span>
          <h2 className="text-2xl font-extrabold tracking-tight">{deviceName}</h2>
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-white/70">
            {info?.battery != null && (
              <span className="tabular flex items-center gap-1">
                <BatteryMedium size={15} aria-hidden />
                {info.battery}%
              </span>
            )}
            {info && (
              <>
                <span aria-hidden>·</span>
                <span className="tabular flex items-center gap-1">
                  <Maximize size={13} aria-hidden />
                  {info.width}×{info.height}
                </span>
              </>
            )}
            {wifiSsid && (
              <>
                <span aria-hidden>·</span>
                <span className="flex items-center gap-1">
                  <Wifi size={14} aria-hidden />
                  {wifiSsid}
                </span>
              </>
            )}
            {lanIp && (
              <>
                <span aria-hidden>·</span>
                <span className="tabular flex items-center gap-1">
                  <Globe size={13} aria-hidden />
                  {lanIp}
                </span>
              </>
            )}
          </div>
          {infoState === "empty" && (
            <p className="mt-1 text-sm text-white/60">
              Add a device from the sidebar to get started.
            </p>
          )}
          {infoState === "error" && (
            <p className="mt-1 text-sm text-danger">{infoError}</p>
          )}

          {/* action row */}
          <div className="mt-4 flex flex-wrap items-center justify-end gap-2">
            <HeroAction
              icon={RefreshCw}
              label="Refresh"
              busy={infoState === "loading"}
              disabled={!hasDevice}
              onClick={refreshInfo}
            />
            <HeroAction
              icon={Power}
              label="Wake"
              busy={action === "wake"}
              disabled={!hasDevice || action !== null}
              onClick={() => runAction("wake", deviceWake, "Device awake")}
            />
            <HeroAction
              icon={Moon}
              label="Sleep"
              busy={action === "sleep"}
              disabled={!hasDevice || action !== null}
              onClick={() => runAction("sleep", deviceSleep, "Device sleeping")}
            />
            <HeroAction
              icon={RotateCcw}
              label="Reboot"
              busy={action === "reboot"}
              disabled={!hasDevice || action !== null}
              onClick={() => setConfirming("reboot")}
            />
            <HeroAction
              icon={Eraser}
              label="Clear"
              busy={action === "clear"}
              disabled={!hasDevice || action !== null}
              onClick={() => setConfirming("clear")}
            />
          </div>
        </div>
      </div>

      {/* General */}
      <Card className="p-5">
        <SectionHeader title="General" />
        <div className="mt-4 flex items-center justify-between gap-3">
          <div>
            <p className="text-sm font-medium text-fg">Launch at login</p>
            <p className="text-xs text-muted">Start this application automatically when you log in.</p>
          </div>
          <Toggle
            checked={autostart}
            disabled={autostartBusy}
            onChange={toggleAutostart}
            label="Launch at login"
          />
        </div>
      </Card>

      {/* Settings editor */}
      <Card className="p-5">
        <SectionHeader title="Device settings" />
        <form onSubmit={saveSettings} className="mt-4 space-y-4">
          <Field label="Name" htmlFor="settings-name">
            <Input
              id="settings-name"
              value={settingsName}
              maxLength={16}
              onChange={(e) => setSettingsName(e.currentTarget.value)}
            />
          </Field>
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
            <Field label="Sleep duration (s)" htmlFor="settings-sleep">
              <Input
                id="settings-sleep"
                type="number"
                value={settingsSleepDuration}
                onChange={(e) => setSettingsSleepDuration(e.currentTarget.value)}
              />
            </Field>
            <Field label="Max idle (s)" htmlFor="settings-max-idle">
              <Input
                id="settings-max-idle"
                type="number"
                value={settingsMaxIdle}
                onChange={(e) => setSettingsMaxIdle(e.currentTarget.value)}
              />
            </Field>
            <Field label="Wake sensitivity" htmlFor="settings-wake-sens">
              <Input
                id="settings-wake-sens"
                type="number"
                value={settingsWakeSens}
                onChange={(e) => setSettingsWakeSens(e.currentTarget.value)}
              />
            </Field>
          </div>
          <div className="flex items-center justify-end gap-3">
            <Button
              type="submit"
              variant="primary"
              disabled={!hasDevice}
              loading={action === "settings"}
            >
              Save settings
            </Button>
          </div>
        </form>
      </Card>

      {confirming === "reboot" && (
        <ConfirmDialog
          title="Reboot device?"
          message="The panel will restart and briefly go offline."
          confirmLabel="Reboot"
          onCancel={() => setConfirming(null)}
          onConfirm={() => {
            setConfirming(null);
            runAction("reboot", deviceReboot, "Rebooting device");
          }}
        />
      )}
      {confirming === "clear" && (
        <ConfirmDialog
          title="Clear screen?"
          message="This wipes the panel to white immediately."
          confirmLabel="Clear"
          onCancel={() => setConfirming(null)}
          onConfirm={() => {
            setConfirming(null);
            runAction("clear", deviceClearScreen, "Screen cleared");
          }}
        />
      )}
    </div>
  );
}

/** Circular translucent action button for the black hero card. */
function HeroAction({
  icon: Icon,
  label,
  busy,
  disabled,
  onClick,
}: {
  icon: typeof Power;
  label: string;
  busy?: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      title={label}
      className={cx(
        "inline-flex h-11 items-center gap-2 rounded-full bg-white/10 px-4 text-sm font-semibold text-inverse-fg transition-all duration-150",
        "hover:bg-white/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/50",
        "disabled:cursor-not-allowed disabled:opacity-40",
      )}
    >
      <Icon size={16} className={busy ? "animate-spin" : undefined} aria-hidden />
      {label}
    </button>
  );
}

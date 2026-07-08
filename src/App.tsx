import { useEffect, useState } from "react";
import {
  CalendarClock,
  Images,
  LayoutGrid,
  Monitor,
  Moon,
  MonitorSmartphone,
  Plus,
  Sun,
} from "lucide-react";
import "./App.css";
import {
  deviceWake,
  errorMessage,
  fetchDeviceInfo,
  getConfig,
  purgeDeviceData,
  saveConfig,
  scheduleDelete,
  schedulesList,
  setActiveDeviceKey,
} from "./api/device";
import DevicePage from "./components/DevicePage";
import DeviceSwitcher, { AddDeviceDialog, type AddDeviceData } from "./components/DeviceSwitcher";
import GalleryPage from "./components/GalleryPage";
import SchedulesPage from "./components/SchedulesPage";
import WidgetsPage from "./components/WidgetsPage";
import { Button, EmptyState, ToastProvider, cx, useToast } from "./components/ui";
import { type ThemeMode, useTheme } from "./lib/theme";
import type { AppConfig } from "./types";

const PAGES = [
  { id: "Device", label: "Device", icon: MonitorSmartphone },
  { id: "Gallery", label: "Gallery", icon: Images },
  { id: "Widgets", label: "Widgets", icon: LayoutGrid },
  { id: "Schedules", label: "Schedules", icon: CalendarClock },
] as const;
type Page = (typeof PAGES)[number]["id"];

const THEME_OPTIONS: { id: ThemeMode; icon: typeof Sun; label: string }[] = [
  { id: "light", icon: Sun, label: "Light" },
  { id: "dark", icon: Moon, label: "Dark" },
  { id: "system", icon: Monitor, label: "System" },
];

function ThemeSwitch() {
  const { mode, setMode } = useTheme();
  return (
    <div className="flex items-center gap-1 rounded-full border border-border bg-surface-2 p-1">
      {THEME_OPTIONS.map((o) => {
        const Icon = o.icon;
        const active = mode === o.id;
        return (
          <button
            key={o.id}
            type="button"
            onClick={() => setMode(o.id)}
            aria-label={`${o.label} theme`}
            aria-pressed={active}
            title={o.label}
            className={cx(
              "flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-all duration-150",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              active ? "bg-primary text-primary-fg" : "text-muted hover:text-fg",
            )}
          >
            <Icon size={15} aria-hidden />
          </button>
        );
      })}
    </div>
  );
}

/** Bloomin8 bloom mark — solid, uses currentColor so it inverts with the
 * theme (white on the black logo tile in light, black in dark). */
function Bloom({ size = 18 }: { size?: number }) {
  const petals = [0, 45, 90, 135, 180, 225, 270, 315];
  return (
    <svg viewBox="0 0 1024 1024" width={size} height={size} fill="currentColor" aria-hidden>
      <g transform="translate(512,512)">
        {petals.map((deg) => (
          <g key={deg} transform={`rotate(${deg})`}>
            <ellipse cx="0" cy="-190" rx="92" ry="188" />
          </g>
        ))}
        <circle cx="0" cy="0" r="96" />
      </g>
    </svg>
  );
}

function newDeviceId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `dev-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

/** Effective active device id: `active_device_id` if it resolves to a real
 * entry, otherwise the first device — mirrors Rust's `AppConfig::active_device`. */
function resolveActiveId(cfg: AppConfig | null): string | null {
  if (!cfg || cfg.devices.length === 0) return null;
  if (cfg.active_device_id && cfg.devices.some((d) => d.id === cfg.active_device_id)) {
    return cfg.active_device_id;
  }
  return cfg.devices[0].id;
}

function App() {
  return (
    <ToastProvider>
      <Shell />
    </ToastProvider>
  );
}

function Shell() {
  const toast = useToast();
  const [page, setPage] = useState<Page>("Device");
  const [config, setConfig] = useState<AppConfig | null>(null);

  const activeId = resolveActiveId(config);

  // The active device's live settings name (`/deviceInfo.name`), kept as the
  // single source of truth so the sidebar switcher matches the Device page
  // hero. Fetched on device switch/startup; the Device page reports fresh names
  // (e.g. after a rename) via `onDeviceName` so the switcher never goes stale.
  const [activeDeviceName, setActiveDeviceName] = useState<string | null>(null);
  const [showAddDevice, setShowAddDevice] = useState(false);
  // The device currently being woken over BLE in the background (device select /
  // startup). Drives the sidebar switcher's "connecting" spinner; cleared when
  // the wake resolves (success or failure — a failed wake just means offline).
  const [wakingId, setWakingId] = useState<string | null>(null);

  /** Persist a freshly-observed `/deviceInfo` name onto the config entry so the
   * stored `name` auto-tracks the device (#6). No-op if unchanged. */
  function persistDeviceName(id: string, name: string) {
    if (!name) return;
    setConfig((prev) => {
      if (!prev) return prev;
      const dev = prev.devices.find((d) => d.id === id);
      if (!dev || dev.name === name) return prev;
      const next = {
        ...prev,
        devices: prev.devices.map((d) => (d.id === id ? { ...d, name } : d)),
      };
      // Fire-and-forget: a persistence failure never breaks the UI.
      saveConfig(next).catch(() => {});
      return next;
    });
  }

  /** Wake `id` over BLE in the background (non-blocking). On success, refresh
   * `/deviceInfo` so the name syncs and the switcher shows the live name. A
   * failed wake just clears the spinner — the device is treated as offline. */
  function backgroundWake(id: string) {
    setWakingId(id);
    deviceWake()
      .then(() => fetchDeviceInfo())
      .then((info) => {
        const name = typeof info.name === "string" ? info.name : "";
        setActiveDeviceName(name || null);
        if (name) persistDeviceName(id, name);
      })
      .catch(() => {})
      .finally(() => setWakingId((cur) => (cur === id ? null : cur)));
  }

  useEffect(() => {
    getConfig()
      .then((cfg) => {
        setConfig(cfg);
        const active = resolveActiveId(cfg);
        setActiveDeviceKey(active ?? "");
        // Kick off a background wake for the initially-active device.
        if (active) backgroundWake(active);
      })
      .catch((e) => toast.show("error", errorMessage(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    let alive = true;
    setActiveDeviceName(null);
    if (!activeId) return;
    fetchDeviceInfo()
      .then((info) => {
        if (!alive) return;
        const name = typeof info.name === "string" ? info.name : null;
        setActiveDeviceName(name);
        if (name) persistDeviceName(activeId, name);
      })
      .catch(() => {
        if (alive) setActiveDeviceName(null);
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeId]);

  /** Persist `next`, update local state, and re-key the image cache. */
  async function applyConfig(next: AppConfig) {
    await saveConfig(next);
    setConfig(next);
    setActiveDeviceKey(resolveActiveId(next) ?? "");
  }

  async function switchDevice(id: string) {
    if (!config || id === activeId) return;
    try {
      await applyConfig({ ...config, active_device_id: id });
      // Auto-wake the newly-selected device in the background (non-blocking).
      backgroundWake(id);
    } catch (e) {
      toast.show("error", errorMessage(e));
    }
  }

  // Errors (duplicate IP, save failure) are thrown so the add dialog can show
  // them inline and stay open; only success closes it + toasts. `data.name` is
  // the real device name the dialog resolved via BLE scan (or the hint/IP when
  // nothing answered); `data.bleHint` is the stored BLE match hint.
  async function addDevice(data: AddDeviceData) {
    const ip = data.lanIp.trim();
    const base = config ?? { devices: [], active_device_id: null };
    if (base.devices.some((d) => d.lan_ip.trim() === ip)) {
      throw new Error("A device with this LAN IP already exists.");
    }
    const hint = data.bleHint.trim() || "Bloomin8";
    const name = data.name.trim() || hint || ip;
    const entry = { id: newDeviceId(), name, lan_ip: ip, ble_name: hint };
    await applyConfig({
      devices: [...base.devices, entry],
      active_device_id: entry.id,
    });
    toast.show("success", "Device added");
    // Wake the just-added device in the background so it comes online promptly.
    backgroundWake(entry.id);
  }

  async function deleteDevice(id: string) {
    if (!config) return;
    const remaining = config.devices.filter((d) => d.id !== id);
    const nextActive =
      id === activeId ? (remaining[0]?.id ?? null) : config.active_device_id;
    try {
      // Clean everything tied to this device: its schedules (so their cron
      // jobs are torn down), then its cached images + last-push record, then
      // the config entry itself.
      const schedules = await schedulesList();
      for (const s of schedules.filter((s) => s.device_id === id)) {
        await scheduleDelete(s.id);
      }
      await purgeDeviceData(id);
      await applyConfig({ devices: remaining, active_device_id: nextActive });
      toast.show("success", "Device removed");
    } catch (e) {
      toast.show("error", errorMessage(e));
    }
  }

  return (
    <div className="flex h-screen bg-bg text-fg">
      <nav className="flex w-56 shrink-0 flex-col border-r border-border bg-surface px-3 py-5">
        <div className="flex items-center gap-2 px-2 pb-6">
          <span className="flex h-8 w-8 items-center justify-center text-fg">
            <Bloom size={26} />
          </span>
          <h1 className="text-lg font-extrabold tracking-tight">Bloomin8</h1>
        </div>

        <ul className="flex-1 space-y-1">
          {PAGES.map(({ id, label, icon: Icon }) => {
            const active = page === id;
            return (
              <li key={id}>
                <button
                  type="button"
                  onClick={() => setPage(id)}
                  aria-current={active ? "page" : undefined}
                  className={cx(
                    "flex w-full cursor-pointer items-center gap-3 rounded-xl px-3 py-2 text-sm font-semibold transition-all duration-150",
                    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    active
                      ? "bg-primary text-primary-fg"
                      : "text-muted hover:bg-surface-2 hover:text-fg",
                  )}
                >
                  <Icon size={18} aria-hidden />
                  {label}
                </button>
              </li>
            );
          })}
        </ul>

        <DeviceSwitcher
          devices={config?.devices ?? []}
          activeId={activeId}
          liveName={activeDeviceName}
          waking={wakingId !== null && wakingId === activeId}
          onSwitch={switchDevice}
          onDelete={deleteDevice}
          onAdd={addDevice}
        />

        <div className="flex items-center justify-between px-1 pt-4">
          <span className="text-xs font-medium text-subtle">Theme</span>
          <ThemeSwitch />
        </div>
      </nav>

      <main className="flex-1 overflow-y-auto">
        {config && config.devices.length === 0 ? (
          // No devices yet — a clean, centered prompt instead of empty pages.
          <div className="flex h-full items-center justify-center p-6">
            <EmptyState
              icon={MonitorSmartphone}
              title="No device yet"
              description="Add your Bloomin8 Canvas by its LAN IP to get started."
              action={
                <Button variant="primary" icon={Plus} onClick={() => setShowAddDevice(true)}>
                  Add device
                </Button>
              }
            />
          </div>
        ) : (
          <>
            {page === "Device" && (
              <DevicePage
                key={activeId ?? "none"}
                onDeviceName={(name) => {
                  setActiveDeviceName(name || null);
                  if (activeId && name) persistDeviceName(activeId, name);
                }}
              />
            )}
            {page === "Gallery" && <GalleryPage key={activeId ?? "none"} />}
            {page === "Widgets" && <WidgetsPage key={activeId ?? "none"} />}
            {page === "Schedules" && <SchedulesPage key={activeId ?? "none"} />}
          </>
        )}
      </main>

      {showAddDevice && (
        <AddDeviceDialog
          existingLanIps={(config?.devices ?? []).map((d) => d.lan_ip.trim())}
          onCancel={() => setShowAddDevice(false)}
          onSubmit={async (data) => {
            await addDevice(data);
            setShowAddDevice(false);
          }}
        />
      )}
    </div>
  );
}

export default App;

import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  Bitcoin,
  CalendarClock,
  Check,
  CloudSun,
  History,
  Image as ImageIcon,
  ImagePlus,
  MapPin,
  Pencil,
  Play,
  Plus,
  Timer,
  Trash2,
} from "lucide-react";
import {
  currentLocation,
  errorMessage,
  getConfig,
  historyList,
  ipGeolocation,
  libraryImage,
  libraryList,
  scheduleDelete,
  scheduleRunNow,
  scheduleSave,
  schedulesList,
  type LibraryItem,
} from "../api/device";
import type {
  AppConfig,
  BorderColor,
  CryptoWidgetConfig,
  DisplayMode,
  HistoryEntry,
  PanelOrientation,
  RotateDirection,
  RunStatus,
  Schedule,
  WidgetConfig,
  WidgetRenderConfig,
} from "../types";
import ConfirmDialog from "./ConfirmDialog";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  Field,
  IconButton,
  Input,
  PillTabs,
  Select,
  Spinner,
  StatusDot,
  Toggle,
  useToast,
} from "./ui";

const MODE_HELP: Record<DisplayMode, string> = {
  auto: "Fill when portrait, fit with a border when landscape.",
  fit: "Show the whole image with a border.",
  fill: "Fill the screen; edges may be cropped.",
};

type AsyncState = "idle" | "busy" | "error" | "success";

function newId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `schedule-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function defaultWidget(kind: WidgetConfig["kind"]): WidgetRenderConfig {
  const orientation: PanelOrientation = "portrait";
  const rotate: RotateDirection = "cw";
  if (kind === "crypto") {
    return { kind: "crypto", symbols: ["BTC"], range: "24h", orientation, rotate };
  }
  if (kind === "countdown") {
    return {
      kind: "countdown",
      target_date: "2026-12-25",
      title: "Countdown",
      bg_query: "landscape",
      bg_photo: null,
      orientation,
      rotate,
    };
  }
  if (kind === "image") {
    return {
      kind: "image",
      library_id: "",
      orientation: "portrait",
      mode: "auto",
      border: "black",
      rotate,
    };
  }
  // Empty (NaN coords / blank label) by default — a new weather schedule
  // defaults to the user's OS location, filled in by ScheduleForm's auto-locate
  // effect. Left empty if permission is denied / unavailable.
  return { kind: "weather", lat: NaN, lon: NaN, city: "", orientation, rotate };
}

function draftSchedule(deviceId: string): Schedule {
  return {
    id: newId(),
    name: "New schedule",
    device_id: deviceId,
    widget: defaultWidget("image"),
    cron: "0 */30 * * * *",
    enabled: true,
  };
}

/** The device the UI is currently acting on, resolved the same way the Rust
 * side does (`AppConfig::active_device`): `active_device_id` if it names a real
 * device, otherwise the first configured one. `null` when no device exists. */
function resolveActiveDeviceId(config: AppConfig): string | null {
  const { devices, active_device_id } = config;
  if (active_device_id && devices.some((d) => d.id === active_device_id)) {
    return active_device_id;
  }
  return devices[0]?.id ?? null;
}

/** A schedule belongs to the active device if it's bound to it, or if it's a
 * legacy schedule with no `device_id` (those migrate to the active device,
 * matching the backend fallback). */
function belongsToDevice(schedule: Schedule, activeId: string | null): boolean {
  if (activeId === null) return false;
  return schedule.device_id === activeId || schedule.device_id === "";
}

/** Per widget kind: the icon + short label shown on a schedule card (the card
 * shows the type as an icon, not the detailed config). */
const WIDGET_META: Record<
  WidgetConfig["kind"],
  { icon: typeof Bitcoin; label: string }
> = {
  crypto: { icon: Bitcoin, label: "Crypto" },
  weather: { icon: CloudSun, label: "Weather" },
  countdown: { icon: Timer, label: "Countdown" },
  image: { icon: ImageIcon, label: "Image" },
};

/** A schedule-run timestamp ("2026-07-08T09:00:02") shown compactly. */
function formatRunTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

// Thumbnails are content-addressed by library id (originals are immutable), so
// cache the decoded data URLs across mounts to avoid re-reading on every edit.
const thumbCache = new Map<string, string>();

/** One selectable library thumbnail — mirrors GalleryPage's tile loading, plus
 * a selected-ring highlight. */
function LibraryThumb({
  item,
  selected,
  onSelect,
}: {
  item: LibraryItem;
  selected: boolean;
  onSelect: () => void;
}) {
  const [src, setSrc] = useState<string | null>(() => thumbCache.get(item.id) ?? null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const cached = thumbCache.get(item.id);
    if (cached) {
      setSrc(cached);
      setFailed(false);
      return;
    }
    let alive = true;
    setSrc(null);
    setFailed(false);
    libraryImage(item.id)
      .then((url) => {
        thumbCache.set(item.id, url);
        if (alive) setSrc(url);
      })
      .catch(() => {
        if (alive) setFailed(true);
      });
    return () => {
      alive = false;
    };
  }, [item.id]);

  return (
    <button
      type="button"
      title={item.name}
      aria-label={`Select ${item.name}`}
      aria-pressed={selected}
      onClick={onSelect}
      className={`relative aspect-square w-20 shrink-0 overflow-hidden rounded-lg bg-surface-2 transition-all focus:outline-none focus-visible:ring-2 focus-visible:ring-ring ${
        selected
          ? "ring-[3px] ring-select ring-offset-2 ring-offset-surface"
          : "ring-1 ring-border hover:ring-fg/30"
      }`}
    >
      {src && <img src={src} alt={item.name} className="h-full w-full object-cover" />}
      {!src && !failed && (
        <div className="absolute inset-0 flex items-center justify-center">
          <Spinner size={18} />
        </div>
      )}
      {failed && (
        <div className="absolute inset-0 flex items-center justify-center text-subtle">
          <ImagePlus size={18} aria-hidden />
        </div>
      )}
      {selected && (
        <>
          <span className="pointer-events-none absolute inset-0 bg-select/25" aria-hidden />
          <span className="absolute right-1 top-1 flex h-5 w-5 items-center justify-center rounded-full bg-select text-white shadow">
            <Check size={13} strokeWidth={3} aria-hidden />
          </span>
        </>
      )}
    </button>
  );
}

/** Library image chooser for image schedules: a grid of selectable thumbnails,
 * or an EmptyState pointing to the Gallery page when the library is empty. */
function LibraryPicker({
  items,
  loading,
  selectedId,
  onSelect,
}: {
  items: LibraryItem[];
  loading: boolean;
  selectedId: string;
  onSelect: (id: string) => void;
}) {
  if (loading && items.length === 0) {
    return (
      <div className="flex items-center justify-center py-8">
        <Spinner size={22} />
      </div>
    );
  }
  if (items.length === 0) {
    return (
      <Card>
        <EmptyState
          icon={ImagePlus}
          title="No images in your library"
          description="Upload images on the Gallery page first, then come back to schedule one."
        />
      </Card>
    );
  }
  return (
    <div className="flex max-h-64 flex-wrap gap-2 overflow-y-auto p-1.5">
      {items.map((item) => (
        <LibraryThumb
          key={item.id}
          item={item}
          selected={item.id === selectedId}
          onSelect={() => onSelect(item.id)}
        />
      ))}
    </div>
  );
}

/** Maps a run status to the shared StatusDot/Badge tone vocabulary. */
function runStatusTone(status: RunStatus): "online" | "offline" | "idle" {
  if (status === "success") return "online";
  if (status === "failed") return "offline";
  return "idle";
}

function runBadgeTone(status: RunStatus): "online" | "danger" | "neutral" {
  if (status === "success") return "online";
  if (status === "failed") return "danger";
  return "neutral";
}

/** Editable form for one schedule's widget config + cron string + enabled
 * toggle. Field set mirrors WidgetsPage's cards but flattened into one form
 * since a schedule is exactly one widget config (v1: raw cron string, no
 * friendly interval picker yet). */
function ScheduleForm({
  initial,
  onCancel,
  onSaved,
}: {
  initial: Schedule;
  onCancel: () => void;
  onSaved: (s: Schedule) => void;
}) {
  const [name, setName] = useState(initial.name);
  const [cron, setCron] = useState(initial.cron);
  const [widget, setWidget] = useState<WidgetRenderConfig>(initial.widget);
  const [cryptoSymbolsText, setCryptoSymbolsText] = useState<string>(
    initial.widget.kind === "crypto" ? initial.widget.symbols.join(", ") : "BTC",
  );
  const [saveState, setSaveState] = useState<AsyncState>("idle");
  const [error, setError] = useState("");
  const [library, setLibrary] = useState<LibraryItem[]>([]);
  const [libraryLoading, setLibraryLoading] = useState(true);
  const [geoState, setGeoState] = useState<AsyncState>("idle");
  // Auto-locate at most once per form so switching kinds/edits never re-prompts.
  const geoAttempted = useRef(false);
  const toast = useToast();

  // Default a weather schedule to the user's OS location: attempt once when the
  // weather surface is shown with no coordinates set yet. Never overrides an
  // existing (edited) location, and leaves the fields empty on denial/timeout.
  useEffect(() => {
    if (widget.kind !== "weather") return;
    if (geoAttempted.current) return;
    if (!Number.isNaN(widget.lat) || !Number.isNaN(widget.lon)) return;
    geoAttempted.current = true;
    let alive = true;
    setGeoState("busy");
    currentLocation()
      .then((loc) => {
        if (!alive) return;
        setWidget((w) =>
          w.kind === "weather"
            ? { ...w, lat: loc.lat, lon: loc.lon, city: loc.city.trim() || w.city }
            : w,
        );
        setGeoState("success");
      })
      .catch(() => {
        if (alive) setGeoState("idle");
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [widget.kind]);

  // Load the local library once so the image picker (and its EmptyState) can
  // render; harmless when no image schedule is being edited.
  useEffect(() => {
    let alive = true;
    setLibraryLoading(true);
    libraryList()
      .then((items) => {
        if (alive) setLibrary(items);
      })
      .catch(() => {
        if (alive) setLibrary([]);
      })
      .finally(() => {
        if (alive) setLibraryLoading(false);
      });
    return () => {
      alive = false;
    };
  }, []);

  function setKind(kind: WidgetConfig["kind"]) {
    if (kind === widget.kind) return;
    const next = defaultWidget(kind);
    setWidget(next);
    if (next.kind === "crypto") setCryptoSymbolsText(next.symbols.join(", "));
  }

  async function onUseLocation() {
    if (widget.kind !== "weather") return;
    setGeoState("busy");
    try {
      // Prefer the precise OS location; fall back to coarse IP geolocation when
      // it's unavailable so the button still does something useful.
      let loc: { lat: number; lon: number; city: string };
      try {
        loc = await currentLocation();
      } catch {
        loc = await ipGeolocation();
      }
      setWidget((w) =>
        w.kind === "weather"
          ? { ...w, lat: loc.lat, lon: loc.lon, city: loc.city.trim() || w.city }
          : w,
      );
      setGeoState("success");
    } catch (e) {
      setGeoState("error");
      toast.show("error", errorMessage(e));
    }
  }

  async function onSave() {
    if (name.trim() === "") return setError("name is required");
    if (cron.trim() === "") return setError("cron expression is required");
    let finalWidget = widget;
    if (finalWidget.kind === "crypto") {
      const symbols = cryptoSymbolsText
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
      if (symbols.length === 0) return setError("at least one symbol is required");
      finalWidget = { ...finalWidget, symbols };
    }
    if (finalWidget.kind === "image" && finalWidget.library_id === "") {
      return setError("select an image from your library");
    }
    if (
      finalWidget.kind === "weather" &&
      (Number.isNaN(finalWidget.lat) || Number.isNaN(finalWidget.lon))
    ) {
      return setError("set a location (latitude and longitude are required)");
    }
    setError("");
    setSaveState("busy");
    try {
      const schedule: Schedule = {
        id: initial.id,
        name: name.trim(),
        // Bound at draft time (create) or preserved from the loaded schedule
        // (edit) — the form never re-targets a schedule's device.
        device_id: initial.device_id,
        widget: finalWidget,
        cron: cron.trim(),
        enabled: initial.enabled, // toggled from the list, not the form
      };
      await scheduleSave(schedule);
      setSaveState("success");
      toast.show("success", "Schedule saved");
      onSaved(schedule);
    } catch (e) {
      setSaveState("error");
      toast.show("error", errorMessage(e));
    }
  }

  return (
    <Card className="space-y-4 p-5">
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <Field label="Name" htmlFor="schedule-name">
          <Input
            id="schedule-name"
            type="text"
            value={name}
            onChange={(e) => setName(e.currentTarget.value)}
          />
        </Field>
        <Field
          label="Cron expression"
          htmlFor="schedule-cron"
        >
          <Input
            id="schedule-cron"
            type="text"
            value={cron}
            onChange={(e) => setCron(e.currentTarget.value)}
            placeholder="0 */30 * * * *"
            className="font-mono"
          />
        </Field>
      </div>

      <Field label="Type" htmlFor="schedule-widget-kind">
        <Select
          id="schedule-widget-kind"
          value={widget.kind}
          onChange={(e) => setKind(e.currentTarget.value as WidgetConfig["kind"])}
        >
          <option value="image">Image</option>
          <option value="weather">Weather</option>
          <option value="crypto">Crypto prices</option>
          <option value="countdown">Countdown</option>
        </Select>
      </Field>

      {widget.kind === "crypto" && (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <Field label="Symbols (comma-separated)" htmlFor="schedule-crypto-symbols">
            <Input
              id="schedule-crypto-symbols"
              type="text"
              value={cryptoSymbolsText}
              onChange={(e) => setCryptoSymbolsText(e.currentTarget.value)}
            />
          </Field>
          <Field label="Range" htmlFor="schedule-crypto-range">
            <Select
              id="schedule-crypto-range"
              value={widget.range}
              onChange={(e) =>
                setWidget({ ...widget, range: e.currentTarget.value as CryptoWidgetConfig["range"] })
              }
            >
              <option value="24h">24h</option>
              <option value="7d">7d</option>
              <option value="30d">30d</option>
            </Select>
          </Field>
        </div>
      )}

      {widget.kind === "weather" && (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
          <Field label="Latitude" htmlFor="schedule-weather-lat">
            <Input
              id="schedule-weather-lat"
              type="text"
              inputMode="decimal"
              value={Number.isNaN(widget.lat) ? "" : widget.lat}
              onChange={(e) => setWidget({ ...widget, lat: Number(e.currentTarget.value) })}
            />
          </Field>
          <Field label="Longitude" htmlFor="schedule-weather-lon">
            <Input
              id="schedule-weather-lon"
              type="text"
              inputMode="decimal"
              value={Number.isNaN(widget.lon) ? "" : widget.lon}
              onChange={(e) => setWidget({ ...widget, lon: Number(e.currentTarget.value) })}
            />
          </Field>
          <Field
            label="City label"
            htmlFor="schedule-weather-city"
            hint={geoState === "busy" ? "Locating…" : undefined}
          >
            <div className="flex items-center gap-2">
              <Input
                id="schedule-weather-city"
                type="text"
                value={widget.city}
                onChange={(e) => setWidget({ ...widget, city: e.currentTarget.value })}
              />
              <IconButton
                icon={MapPin}
                label="Use my location"
                loading={geoState === "busy"}
                onClick={onUseLocation}
              />
            </div>
          </Field>
        </div>
      )}

      {widget.kind === "countdown" && (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <Field label="Target date" htmlFor="schedule-countdown-date">
            <Input
              id="schedule-countdown-date"
              type="date"
              value={widget.target_date}
              onChange={(e) => setWidget({ ...widget, target_date: e.currentTarget.value })}
            />
          </Field>
          <Field label="Title" htmlFor="schedule-countdown-title">
            <Input
              id="schedule-countdown-title"
              type="text"
              value={widget.title}
              onChange={(e) => setWidget({ ...widget, title: e.currentTarget.value })}
            />
          </Field>
        </div>
      )}

      {widget.kind === "image" && (
        <div className="space-y-1.5">
          <p className="text-sm font-medium text-fg">Image</p>
          <LibraryPicker
            items={library}
            loading={libraryLoading}
            selectedId={widget.library_id}
            onSelect={(library_id) => setWidget({ ...widget, library_id })}
          />
        </div>
      )}

      {/* Orientation applies to every schedule type (widgets render swapped +
          rotated for landscape; images are fit/filled per the panel). */}
      <div className="space-y-1.5">
        <p className="text-sm font-medium text-fg">Orientation</p>
        <PillTabs
          tabs={[
            { id: "portrait", label: "Portrait" },
            { id: "landscape", label: "Landscape" },
          ]}
          active={widget.orientation}
          onChange={(orientation: PanelOrientation) => setWidget({ ...widget, orientation })}
        />
      </div>

      {/* Fit/fill + border only make sense for an arbitrary-aspect image. */}
      {widget.kind === "image" && (
        <>
          <div className="space-y-1.5">
            <p className="text-sm font-medium text-fg">Display mode</p>
            <PillTabs
              tabs={[
                { id: "auto", label: "Auto" },
                { id: "fit", label: "Fit" },
                { id: "fill", label: "Fill" },
              ]}
              active={widget.mode}
              onChange={(mode: DisplayMode) => setWidget({ ...widget, mode })}
            />
            <p className="text-xs text-muted">{MODE_HELP[widget.mode]}</p>
          </div>

          <div className="space-y-1.5">
            <p className={`text-sm font-medium ${widget.mode === "fill" ? "text-subtle" : "text-fg"}`}>
              Border color{widget.mode === "fill" ? " (not used in fill mode)" : ""}
            </p>
            <div className={widget.mode === "fill" ? "pointer-events-none opacity-40" : undefined}>
              <PillTabs
                tabs={[
                  { id: "white", label: "White" },
                  { id: "black", label: "Black" },
                ]}
                active={widget.border}
                onChange={(border: BorderColor) => setWidget({ ...widget, border })}
              />
            </div>
          </div>
        </>
      )}

      {error && <p className="text-sm text-danger" role="alert">{error}</p>}

      <div className="flex justify-end gap-2">
        <Button type="button" variant="secondary" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="button" variant="primary" onClick={onSave} loading={saveState === "busy"}>
          Save
        </Button>
      </div>
    </Card>
  );
}

function ScheduleRow({
  schedule,
  lastRun,
  onEdit,
  onDeleted,
  onRan,
  onChanged,
}: {
  schedule: Schedule;
  lastRun: HistoryEntry | undefined;
  onEdit: () => void;
  onDeleted: () => void;
  onRan: (entry: HistoryEntry) => void;
  onChanged: () => void;
}) {
  const widgetMeta = WIDGET_META[schedule.widget.kind];
  const WidgetIcon = widgetMeta.icon;
  const [runState, setRunState] = useState<AsyncState>("idle");
  const [deleteState, setDeleteState] = useState<AsyncState>("idle");
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [toggling, setToggling] = useState(false);
  const toast = useToast();

  async function onToggleEnabled(next: boolean) {
    setToggling(true);
    try {
      await scheduleSave({ ...schedule, enabled: next });
      onChanged();
    } catch (e) {
      toast.show("error", errorMessage(e));
    } finally {
      setToggling(false);
    }
  }

  async function onRunNow() {
    setRunState("busy");
    try {
      const entry = await scheduleRunNow(schedule.id);
      setRunState(entry.status === "success" ? "success" : "error");
      if (entry.status === "success") {
        toast.show("success", `Ran ${schedule.name}`);
      } else {
        toast.show("error", entry.error ?? entry.status);
      }
      onRan(entry);
    } catch (e) {
      setRunState("error");
      toast.show("error", errorMessage(e));
    }
  }

  async function onDelete() {
    setDeleteState("busy");
    try {
      await scheduleDelete(schedule.id);
      onDeleted();
    } catch (e) {
      setDeleteState("error");
      toast.show("error", errorMessage(e));
    }
  }

  return (
    <Card className="flex h-full flex-col gap-3 p-4">
      <div className="flex items-start gap-3">
        <span
          className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-surface-2 text-fg"
          title={widgetMeta.label}
          aria-label={widgetMeta.label}
        >
          <WidgetIcon size={18} aria-hidden />
        </span>
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-bold text-fg">{schedule.name}</p>
          <p className="truncate font-mono text-xs text-muted">{schedule.cron}</p>
        </div>
      </div>

      <div className="flex items-center gap-1.5 text-xs">
        {lastRun ? (
          <>
            <StatusDot tone={runStatusTone(lastRun.status)} />
            <span className="text-muted">
              Last run · <span className="tabular text-fg">{formatRunTime(lastRun.finished_at)}</span>
            </span>
          </>
        ) : (
          <span className="text-subtle">Never run</span>
        )}
      </div>

      <div className="mt-auto flex items-center justify-between gap-2 border-t border-border pt-3">
        <div className="flex items-center gap-2">
          <Toggle
            checked={schedule.enabled}
            onChange={onToggleEnabled}
            disabled={toggling}
            label={schedule.enabled ? "Disable schedule" : "Enable schedule"}
          />
        </div>
        <div className="flex items-center gap-1.5">
          <IconButton
            icon={Play}
            label="Run now"
            variant="secondary"
            size="sm"
            onClick={onRunNow}
            loading={runState === "busy"}
          />
          <IconButton icon={Pencil} label="Edit schedule" variant="ghost" size="sm" onClick={onEdit} />
          <IconButton
            icon={Trash2}
            label="Delete schedule"
            variant="danger"
            size="sm"
            onClick={() => setConfirmingDelete(true)}
            loading={deleteState === "busy"}
          />
        </div>
      </div>

      {confirmingDelete && (
        <ConfirmDialog
          title="Delete schedule?"
          message={`"${schedule.name}" will stop running and be removed permanently.`}
          confirmLabel="Delete"
          onCancel={() => setConfirmingDelete(false)}
          onConfirm={() => {
            setConfirmingDelete(false);
            onDelete();
          }}
        />
      )}
    </Card>
  );
}

function HistoryLog({
  history,
  nameFor,
}: {
  history: HistoryEntry[];
  nameFor: (id: string) => string;
}) {
  if (history.length === 0) {
    return (
      <p className="px-1 text-sm text-muted">No runs recorded yet.</p>
    );
  }
  return (
    <Card className="max-h-64 divide-y divide-border overflow-y-auto">
      {history.map((entry, i) => (
        <div key={i} className="flex items-center gap-3 px-4 py-2.5">
          <StatusDot tone={runStatusTone(entry.status)} />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-x-2">
              <span className="truncate text-xs font-semibold text-fg">{nameFor(entry.schedule_id)}</span>
              <span className="tabular text-xs text-muted">{entry.started_at}</span>
            </div>
            {(entry.filename ?? entry.error) && (
              <p
                className={`truncate text-xs ${entry.status === "failed" ? "text-danger" : "text-muted"}`}
              >
                {entry.filename ?? entry.error}
              </p>
            )}
          </div>
          <Badge tone={runBadgeTone(entry.status)}>{entry.status}</Badge>
        </div>
      ))}
    </Card>
  );
}

export default function SchedulesPage() {
  const [schedules, setSchedules] = useState<Schedule[]>([]);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [listState, setListState] = useState<AsyncState>("idle");
  const [listError, setListError] = useState("");
  const [editing, setEditing] = useState<Schedule | null>(null);
  const [creating, setCreating] = useState(false);
  const [view, setView] = useState<"list" | "history">("list");
  // The device this page is scoped to. `null` = no device configured yet.
  // The page is remounted (App keys it on the active id) when the user switches
  // devices, so a fresh `getConfig()`/`schedulesList()` here reflects the new
  // device — no need to subscribe to switch events.
  const [activeId, setActiveId] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setListState("busy");
    try {
      const [config, s, h] = await Promise.all([
        getConfig(),
        schedulesList(),
        historyList(200),
      ]);
      setActiveId(resolveActiveDeviceId(config));
      setSchedules(s);
      setHistory(h);
      setListState("success");
    } catch (e) {
      setListState("error");
      setListError(errorMessage(e));
    }
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  // Everything the page shows is scoped to the active device: its schedules,
  // and the history of just those schedules.
  const visibleSchedules = schedules.filter((s) => belongsToDevice(s, activeId));
  const visibleIds = new Set(visibleSchedules.map((s) => s.id));
  const visibleHistory = history.filter((h) => visibleIds.has(h.schedule_id));

  const lastRunFor = (id: string) => history.find((h) => h.schedule_id === id);
  const nameFor = (id: string) => schedules.find((s) => s.id === id)?.name ?? "(deleted schedule)";

  // Create/edit is a separate view — it replaces the list rather than growing
  // an inline form above it.
  if (creating || editing) {
    const closeForm = () => {
      setCreating(false);
      setEditing(null);
    };
    return (
      <div className="mx-auto max-w-3xl space-y-5 p-6">
        <div className="flex items-center gap-3">
          <IconButton icon={ArrowLeft} label="Back to schedules" onClick={closeForm} />
          <h1 className="text-3xl font-extrabold tracking-tight text-fg">
            {editing ? "Edit schedule" : "New schedule"}
          </h1>
        </div>
        <ScheduleForm
          initial={editing ?? draftSchedule(activeId ?? "")}
          onCancel={closeForm}
          onSaved={() => {
            closeForm();
            reload();
          }}
        />
      </div>
    );
  }

  if (view === "history") {
    return (
      <div className="mx-auto max-w-3xl space-y-5 p-6">
        <div className="flex items-center gap-3">
          <IconButton icon={ArrowLeft} label="Back to schedules" onClick={() => setView("list")} />
          <h1 className="text-3xl font-extrabold tracking-tight text-fg">Run history</h1>
        </div>
        <HistoryLog history={visibleHistory} nameFor={nameFor} />
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-5xl space-y-5 p-6">
      <div className="flex items-center justify-between gap-3">
        <h1 className="text-3xl font-extrabold tracking-tight text-fg">Schedules</h1>
        <div className="flex items-center gap-2">
          <IconButton icon={History} label="Run history" onClick={() => setView("history")} />
          {activeId !== null && (
            <Button type="button" variant="primary" pill icon={Plus} onClick={() => setCreating(true)}>
              New schedule
            </Button>
          )}
        </div>
      </div>

      {listState === "error" && (
        <Card className="flex items-center justify-between gap-3 p-4">
          <p className="text-sm text-danger">{listError}</p>
          <Button type="button" variant="secondary" size="sm" onClick={reload}>
            Retry
          </Button>
        </Card>
      )}

      {listState === "busy" && visibleSchedules.length === 0 && (
        <div className="flex items-center justify-center py-12">
          <Spinner size={24} />
        </div>
      )}

      {activeId === null && listState === "success" && (
        <Card>
          <EmptyState
            icon={CalendarClock}
            title="No device yet"
            description="Schedules run on a specific device. Add a device from the sidebar first, then come back to schedule automatic refreshes for it."
          />
        </Card>
      )}

      {activeId !== null && visibleSchedules.length === 0 && listState === "success" && (
        <Card>
          <EmptyState
            icon={CalendarClock}
            title="No schedules yet"
            description="Schedules run a widget on this device automatically — set a cron expression and pick a widget to get started."
            action={
              <Button
                type="button"
                variant="primary"
                pill
                icon={Plus}
                onClick={() => setCreating(true)}
              >
                Create your first schedule
              </Button>
            }
          />
        </Card>
      )}

      {visibleSchedules.length > 0 && (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {visibleSchedules.map((s) => (
            <ScheduleRow
              key={s.id}
              schedule={s}
              lastRun={lastRunFor(s.id)}
              onEdit={() => setEditing(s)}
              onDeleted={reload}
              onRan={(entry) => setHistory((prev) => [entry, ...prev])}
              onChanged={reload}
            />
          ))}
        </div>
      )}
    </div>
  );
}

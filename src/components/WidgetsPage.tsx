import { useEffect, useState } from "react";
import {
  Bitcoin,
  CalendarClock,
  CloudSun,
  Eye,
  Image as ImageIcon,
  MapPin,
  Send,
} from "lucide-react";
import {
  currentLocation,
  errorMessage,
  ipGeolocation,
  previewWidget,
  pushWidget,
} from "../api/device";
import type {
  CountdownWidgetConfig,
  CryptoWidgetConfig,
  PanelOrientation,
  WeatherWidgetConfig,
  WidgetRenderConfig,
} from "../types";
import { Button, Card, Field, IconButton, Input, PillTabs, Select, SectionHeader, Spinner, useToast, cx } from "./ui";

type AsyncState = "idle" | "busy" | "error" | "success";

/** Orientation picker shared by all three widget cards. */
function OrientationControls({
  orientation,
  setOrientation,
  idPrefix,
}: {
  orientation: PanelOrientation;
  setOrientation: (o: PanelOrientation) => void;
  idPrefix: string;
}) {
  return (
    <div className="space-y-4">
      <Field label="Orientation" htmlFor={`${idPrefix}-orientation`}>
        <PillTabs
          tabs={[
            { id: "portrait", label: "Portrait" },
            { id: "landscape", label: "Landscape" },
          ]}
          active={orientation}
          onChange={setOrientation}
        />
      </Field>
    </div>
  );
}

/** Preview/push state + rendered nodes, shared by all three cards. Returns the
 * preview box (for the right column) and the action buttons (for a full-width
 * row at the card's bottom) separately, so buttons aren't cramped next to the
 * preview. The preview box follows the *previewed* orientation, so previewing a
 * landscape widget flips the box to landscape. */
function usePreviewPush(buildConfig: () => WidgetRenderConfig | string, idPrefix: string) {
  const [previewState, setPreviewState] = useState<AsyncState>("idle");
  const [previewSrc, setPreviewSrc] = useState<string | null>(null);
  const [previewOrientation, setPreviewOrientation] = useState<PanelOrientation>("portrait");
  const [pushState, setPushState] = useState<AsyncState>("idle");
  const toast = useToast();

  function resolveConfig(): WidgetRenderConfig | null {
    const result = buildConfig();
    if (typeof result === "string") {
      toast.show("error", result);
      return null;
    }
    return result;
  }

  async function onPreview() {
    const cfg = resolveConfig();
    if (!cfg) return;
    setPreviewState("busy");
    try {
      const dataUrl = await previewWidget(cfg);
      setPreviewSrc(dataUrl);
      setPreviewOrientation(cfg.orientation);
      setPreviewState("success");
    } catch (e) {
      setPreviewState(previewSrc ? "success" : "idle");
      toast.show("error", errorMessage(e));
    }
  }

  async function onPush() {
    const cfg = resolveConfig();
    if (!cfg) return;
    setPushState("busy");
    try {
      const filename = await pushWidget(cfg);
      setPushState("idle");
      toast.show("success", `Pushed to device · ${filename}`);
    } catch (e) {
      setPushState("idle");
      toast.show("error", errorMessage(e));
    }
  }

  const busy = previewState === "busy" || pushState === "busy";

  const preview = (
    <div
      className={cx(
        "flex w-full items-center justify-center overflow-hidden rounded-2xl border border-border bg-surface-2",
        previewOrientation === "landscape" ? "aspect-[4/3]" : "aspect-[3/4]",
      )}
      data-testid={previewState === "busy" && !previewSrc ? `${idPrefix}-preview-spinner` : undefined}
    >
      {previewState === "busy" && !previewSrc && <Spinner size={22} />}
      {previewSrc && (
        <img src={previewSrc} alt={`${idPrefix} preview`} className="h-full w-full object-contain" />
      )}
      {previewState !== "busy" && !previewSrc && (
        <div className="flex flex-col items-center gap-1.5 text-subtle">
          <ImageIcon size={22} aria-hidden />
          <span className="text-xs">No preview yet</span>
        </div>
      )}
    </div>
  );

  // Equal-width buttons in a 2-col grid so their size stays fixed regardless of
  // label/loading state (no width jump on disable).
  const actions = (
    <div className="grid grid-cols-2 gap-2">
      <Button
        type="button"
        variant="secondary"
        icon={Eye}
        loading={previewState === "busy"}
        disabled={busy}
        onClick={onPreview}
        className="w-full"
      >
        {previewState === "busy" ? "Rendering…" : "Preview"}
      </Button>
      <Button
        type="button"
        variant="primary"
        icon={Send}
        loading={pushState === "busy"}
        disabled={busy}
        onClick={onPush}
        className="w-full"
      >
        {pushState === "busy" ? "Pushing…" : "Push"}
      </Button>
    </div>
  );

  return { preview, actions };
}

function WidgetCard({
  title,
  icon,
  settings,
  preview,
  actions,
}: {
  title: string;
  icon: typeof Bitcoin;
  settings: React.ReactNode;
  preview: React.ReactNode;
  actions: React.ReactNode;
}) {
  return (
    <Card className="space-y-4 p-5">
      <SectionHeader icon={icon} title={title} />
      {/* Horizontal layout: settings ~60% left, preview ~40% right. Uses a
          container query (not viewport) so it stays side-by-side whenever the
          card itself is wide enough — the sidebar width no longer forces it to
          stack at the default window size. Only stacks when the card is truly
          narrow. */}
      <div className="@container">
        <div className="grid grid-cols-1 gap-6 @lg:grid-cols-5">
          <div className="space-y-4 @lg:col-span-3">{settings}</div>
          <div className="@lg:col-span-2">{preview}</div>
        </div>
      </div>
      {/* Preview/Push span the full card width along the bottom. */}
      {actions}
    </Card>
  );
}

// --- Crypto ----------------------------------------------------------------

function CryptoCard() {
  const [symbolsInput, setSymbolsInput] = useState("BTC, ETH");
  const [range, setRange] = useState<CryptoWidgetConfig["range"]>("24h");
  const [orientation, setOrientation] = useState<PanelOrientation>("portrait");

  function buildConfig(): WidgetRenderConfig | string {
    const symbols = symbolsInput
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    if (symbols.length === 0) {
      return "enter at least one symbol (comma-separated), e.g. BTC, ETH";
    }
    return { kind: "crypto", symbols, range, orientation, rotate: "cw" };
  }

  const { preview, actions } = usePreviewPush(buildConfig, "crypto");

  return (
    <WidgetCard
      title="Crypto prices"
      icon={Bitcoin}
      settings={
        <>
          <Field
            label="Symbols (comma-separated)"
            htmlFor="crypto-symbols"
          >
            <Input
              id="crypto-symbols"
              type="text"
              value={symbolsInput}
              onChange={(e) => setSymbolsInput(e.currentTarget.value)}
              placeholder="BTC, ETH"
            />
          </Field>
          <Field label="Range" htmlFor="crypto-range">
            <Select
              id="crypto-range"
              value={range}
              onChange={(e) => setRange(e.currentTarget.value as CryptoWidgetConfig["range"])}
            >
              <option value="24h">24h</option>
              <option value="7d">7d</option>
              <option value="30d">30d</option>
            </Select>
          </Field>
          <OrientationControls
            idPrefix="crypto"
            orientation={orientation}
            setOrientation={setOrientation}
          />
        </>
      }
      preview={preview}
      actions={actions}
    />
  );
}

// --- Weather -----------------------------------------------------------------

function WeatherCard() {
  // Empty by default — the widget defaults to the user's OS location, filled in
  // by the auto-locate effect below. If that fails (permission denied /
  // unavailable) the fields stay empty and buildConfig surfaces the usual
  // "latitude/longitude must be a number" error at push time.
  const [lat, setLat] = useState("");
  const [lon, setLon] = useState("");
  const [city, setCity] = useState("");
  const [orientation, setOrientation] = useState<PanelOrientation>("portrait");
  const [geoState, setGeoState] = useState<AsyncState>("idle");
  const [geoError, setGeoError] = useState("");

  // Auto-default to the OS location once on mount, but only when nothing is set
  // yet — never overrides a value the user typed, and never re-prompts.
  useEffect(() => {
    if (lat !== "" || lon !== "") return;
    let alive = true;
    setGeoState("busy");
    currentLocation()
      .then((loc) => {
        if (!alive) return;
        setLat(String(loc.lat));
        setLon(String(loc.lon));
        if (loc.city.trim()) setCity(loc.city.trim());
        setGeoState("success");
      })
      .catch(() => {
        // Denied / unavailable / timeout: leave the fields empty, don't block.
        if (alive) setGeoState("idle");
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function onUseLocation() {
    setGeoState("busy");
    setGeoError("");
    try {
      // Prefer the precise OS location; fall back to coarse IP geolocation when
      // the OS location is unavailable (e.g. permission denied) so the button
      // still does something useful.
      let loc: { lat: number; lon: number; city: string };
      try {
        loc = await currentLocation();
      } catch {
        loc = await ipGeolocation();
      }
      setLat(String(loc.lat));
      setLon(String(loc.lon));
      if (loc.city.trim()) setCity(loc.city.trim());
      setGeoState("success");
    } catch (e) {
      setGeoState("error");
      setGeoError(errorMessage(e));
    }
  }

  function buildConfig(): WidgetRenderConfig | string {
    const latNum = Number(lat);
    const lonNum = Number(lon);
    if (lat.trim() === "" || Number.isNaN(latNum)) return "latitude must be a number";
    if (lon.trim() === "" || Number.isNaN(lonNum)) return "longitude must be a number";
    // City is a cosmetic label — weather renders from lat/lon alone, so an
    // empty label is allowed (e.g. reverse geocoding failed).
    const cfg: WeatherWidgetConfig = { kind: "weather", lat: latNum, lon: lonNum, city: city.trim() };
    return { ...cfg, orientation, rotate: "cw" };
  }

  const { preview, actions } = usePreviewPush(buildConfig, "weather");

  return (
    <WidgetCard
      title="Weather"
      icon={CloudSun}
      settings={
        <>
          <div className="grid grid-cols-2 gap-3">
            <Field label="Latitude" htmlFor="weather-lat">
              <Input
                id="weather-lat"
                type="text"
                inputMode="decimal"
                value={lat}
                onChange={(e) => setLat(e.currentTarget.value)}
              />
            </Field>
            <Field label="Longitude" htmlFor="weather-lon">
              <Input
                id="weather-lon"
                type="text"
                inputMode="decimal"
                value={lon}
                onChange={(e) => setLon(e.currentTarget.value)}
              />
            </Field>
          </div>
          <Field
            label="City label"
            htmlFor="weather-city"
            error={geoState === "error" ? geoError : undefined}
            hint={geoState === "busy" ? "Locating…" : undefined}
          >
            <div className="flex items-center gap-2">
              <Input
                id="weather-city"
                type="text"
                value={city}
                onChange={(e) => setCity(e.currentTarget.value)}
              />
              <IconButton
                icon={MapPin}
                label="Use my location"
                loading={geoState === "busy"}
                onClick={onUseLocation}
              />
            </div>
          </Field>
          <OrientationControls
            idPrefix="weather"
            orientation={orientation}
            setOrientation={setOrientation}
          />
        </>
      }
      preview={preview}
      actions={actions}
    />
  );
}

// --- Countdown ---------------------------------------------------------------

function CountdownCard() {
  const [targetDate, setTargetDate] = useState("2026-12-25");
  const [title, setTitle] = useState("Christmas");
  const [bgQuery, setBgQuery] = useState("cat");
  const [bgPhoto, setBgPhoto] = useState("");
  const [orientation, setOrientation] = useState<PanelOrientation>("portrait");

  function buildConfig(): WidgetRenderConfig | string {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(targetDate)) return "target date must be YYYY-MM-DD";
    if (title.trim() === "") return "title is required";
    const cfg: CountdownWidgetConfig = {
      kind: "countdown",
      target_date: targetDate,
      title: title.trim(),
      bg_query: bgQuery.trim(),
      bg_photo: bgPhoto.trim() || null,
    };
    return { ...cfg, orientation, rotate: "cw" };
  }

  const { preview, actions } = usePreviewPush(buildConfig, "countdown");

  return (
    <WidgetCard
      title="Countdown"
      icon={CalendarClock}
      settings={
        <>
          <div className="grid grid-cols-2 gap-3">
            <Field label="Target date" htmlFor="countdown-date">
              <Input
                id="countdown-date"
                type="date"
                value={targetDate}
                onChange={(e) => setTargetDate(e.currentTarget.value)}
              />
            </Field>
            <Field label="Title" htmlFor="countdown-title">
              <Input
                id="countdown-title"
                type="text"
                value={title}
                onChange={(e) => setTitle(e.currentTarget.value)}
              />
            </Field>
          </div>
          <Field
            label="Background art search (Met Museum)"
            htmlFor="countdown-bg-query"
            hint={bgPhoto.trim() !== "" ? "Disabled while a local photo path is set" : undefined}
          >
            <Input
              id="countdown-bg-query"
              type="text"
              value={bgQuery}
              onChange={(e) => setBgQuery(e.currentTarget.value)}
              placeholder="van gogh landscape"
              disabled={bgPhoto.trim() !== ""}
            />
          </Field>
          <Field
            label="Local photo path (optional — skips the art search/network)"
            htmlFor="countdown-bg-photo"
          >
            <Input
              id="countdown-bg-photo"
              type="text"
              value={bgPhoto}
              onChange={(e) => setBgPhoto(e.currentTarget.value)}
              placeholder="/Users/me/Pictures/anniversary.jpg"
            />
          </Field>
          <OrientationControls
            idPrefix="countdown"
            orientation={orientation}
            setOrientation={setOrientation}
          />
        </>
      }
      preview={preview}
      actions={actions}
    />
  );
}

type WidgetTab = "crypto" | "weather" | "countdown";

const WIDGET_TABS: { id: WidgetTab; label: string }[] = [
  { id: "crypto", label: "Crypto" },
  { id: "weather", label: "Weather" },
  { id: "countdown", label: "Countdown" },
];

export default function WidgetsPage() {
  const [active, setActive] = useState<WidgetTab>("crypto");
  return (
    <div className="mx-auto max-w-3xl space-y-5 p-6">
      <h1 className="text-3xl font-extrabold tracking-tight text-fg">Widgets</h1>
      <PillTabs tabs={WIDGET_TABS} active={active} onChange={setActive} />
      {/* All three stay mounted (hidden when inactive) so switching sub-pages
          preserves each widget's form input and preview. */}
      <div className={cx(active !== "crypto" && "hidden")}>
        <CryptoCard />
      </div>
      <div className={cx(active !== "weather" && "hidden")}>
        <WeatherCard />
      </div>
      <div className={cx(active !== "countdown" && "hidden")}>
        <CountdownCard />
      </div>
    </div>
  );
}

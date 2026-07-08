import { useEffect, useState } from "react";
import { Send } from "lucide-react";
import {
  errorMessage,
  libraryImage,
  pushImage,
  type BorderColor,
  type DisplayMode,
  type DisplaySettings,
} from "../api/device";
import { Button, PillTabs, Spinner, useToast } from "./ui";

type SourceState = "loading" | "loaded" | "error";

const DEFAULT_SETTINGS: DisplaySettings = {
  orientation: "portrait",
  rotate: "cw",
  mode: "auto",
  border: "black",
};

const MODE_HELP: Record<DisplayMode, string> = {
  auto: "Fill when portrait, fit with a border when landscape.",
  fit: "Show the whole image with a border.",
  fill: "Fill the screen; edges may be cropped.",
};

/** How the image sits in the panel, resolved for the CSS preview. `auto`
 * matches the backend: fill (cover) when portrait, fit (contain) when
 * landscape. This is a pure visual approximation — the real panel-sized JPEG is
 * only generated on push (`pushImage`), never for the live preview. */
function resolveObjectFit(mode: DisplayMode, orientation: DisplaySettings["orientation"]) {
  if (mode === "fill") return "object-cover";
  if (mode === "fit") return "object-contain";
  return orientation === "portrait" ? "object-cover" : "object-contain";
}

/**
 * Push-settings dialog: pick display settings for one image, preview the
 * result with pure CSS (no backend round-trip), then push it to the device —
 * the real panel-sized JPEG is only rendered when the user confirms. Accepts
 * either a local library id (resolved via `libraryImage`) or a raw data-URL
 * `source` directly.
 */
export default function PushImageDialog({
  libraryId,
  source: sourceProp,
  onClose,
}: {
  libraryId?: string;
  source?: string;
  onClose: () => void;
}) {
  const toast = useToast();

  const [source, setSource] = useState<string | null>(sourceProp ?? null);
  const [sourceState, setSourceState] = useState<SourceState>(sourceProp ? "loaded" : "loading");
  const [sourceError, setSourceError] = useState("");

  const [settings, setSettings] = useState<DisplaySettings>(DEFAULT_SETTINGS);
  const [pushing, setPushing] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    if (sourceProp) return;
    if (!libraryId) return;
    let alive = true;
    setSourceState("loading");
    setSourceError("");
    libraryImage(libraryId)
      .then((url) => {
        if (!alive) return;
        setSource(url);
        setSourceState("loaded");
      })
      .catch((e) => {
        if (!alive) return;
        setSourceState("error");
        setSourceError(errorMessage(e));
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [libraryId, sourceProp]);

  async function handlePush() {
    if (!source) return;
    setPushing(true);
    try {
      const filename = await pushImage(source, settings);
      toast.show("success", `Pushed to device · ${filename}`);
      onClose();
    } catch (e) {
      toast.show("error", errorMessage(e));
    } finally {
      setPushing(false);
    }
  }

  // The preview box takes the chosen panel orientation (portrait 3:4 = 12:16 /
  // landscape 4:3 = 16:12). Cap the *long edge* (the panel's 1600px side) so
  // both orientations render at the same visual size — capping width instead
  // would make the landscape preview look small.
  const previewSize =
    settings.orientation === "portrait"
      ? "h-[240px] aspect-[3/4]"
      : "w-[240px] max-w-full aspect-[4/3]";
  const objectFit = resolveObjectFit(settings.mode, settings.orientation);
  const borderBg = settings.border === "white" ? "#ffffff" : "#000000";

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
    >
      <div
        className="flex max-h-[90vh] w-full max-w-md flex-col gap-5 overflow-y-auto rounded-2xl border border-border bg-surface p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="text-base font-bold text-fg">Push to device</h3>

        {/* Picture-frame preview: thin black frame -> generous white mat -> the
            panel image, mirroring the physical framed Canvas (proportions per
            the official app: slim frame, wide mat). The wrapper is fixed to the
            tallest (portrait) frame height so switching orientation re-centers
            the frame instead of resizing the dialog / shoving the options. */}
        <div className="flex h-[336px] items-center justify-center">
          {/* A hairline in the theme's border colour keeps the black frame
              visible against the dark dialog background in dark mode. */}
          <div
            className="w-fit rounded-sm p-2 shadow-xl ring-1 ring-border"
            style={{ backgroundColor: "#000000" }}
          >
            <div className="p-10" style={{ backgroundColor: "#ffffff" }}>
            <div
              className={`relative overflow-hidden ${previewSize}`}
              style={{ backgroundColor: sourceState === "loaded" ? borderBg : "#e5e5e5" }}
            >
              {sourceState === "error" && (
                <div className="absolute inset-0 flex items-center justify-center bg-surface-2 p-4 text-center text-sm text-danger">
                  {sourceError}
                </div>
              )}
              {source && sourceState === "loaded" && (
                <img src={source} alt="Preview" className={`h-full w-full ${objectFit}`} />
              )}
              {sourceState === "loading" && (
                <div className="absolute inset-0 flex items-center justify-center bg-surface-2">
                  <Spinner size={24} />
                </div>
              )}
            </div>
            </div>
          </div>
        </div>

        <div className="space-y-1.5">
          <p className="text-sm font-medium text-fg">Orientation</p>
          <PillTabs
            tabs={[
              { id: "portrait", label: "Portrait" },
              { id: "landscape", label: "Landscape" },
            ]}
            active={settings.orientation}
            onChange={(orientation) => setSettings((s) => ({ ...s, orientation }))}
          />
        </div>

        <div className="space-y-1.5">
          <p className="text-sm font-medium text-fg">Display mode</p>
          <PillTabs
            tabs={[
              { id: "auto", label: "Auto" },
              { id: "fit", label: "Fit" },
              { id: "fill", label: "Fill" },
            ]}
            active={settings.mode}
            onChange={(mode) => setSettings((s) => ({ ...s, mode }))}
          />
          <p className="text-xs text-muted">{MODE_HELP[settings.mode]}</p>
        </div>

        <div className="space-y-1.5">
          <p className={`text-sm font-medium ${settings.mode === "fill" ? "text-subtle" : "text-fg"}`}>
            Border color{settings.mode === "fill" ? " (not used in fill mode)" : ""}
          </p>
          <div className={settings.mode === "fill" ? "pointer-events-none opacity-40" : undefined}>
            <PillTabs
              tabs={[
                { id: "white", label: "White" },
                { id: "black", label: "Black" },
              ]}
              active={settings.border}
              onChange={(border: BorderColor) => setSettings((s) => ({ ...s, border }))}
            />
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-border pt-4">
          <Button variant="ghost" onClick={onClose} disabled={pushing}>
            Cancel
          </Button>
          <Button
            variant="primary"
            icon={Send}
            loading={pushing}
            disabled={!source || sourceState !== "loaded"}
            onClick={() => void handlePush()}
          >
            Push
          </Button>
        </div>
      </div>
    </div>
  );
}

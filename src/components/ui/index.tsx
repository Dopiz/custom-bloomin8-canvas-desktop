/**
 * Shared UI primitives for the Bloomin8 desktop app.
 * See design-system/MASTER.md for the visual DNA and token contract.
 * Components reference semantic token utilities (bg-surface, text-fg, …) only.
 */
import {
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";
import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  ImageOff,
  Loader2,
  type LucideIcon,
} from "lucide-react";
import { cachedImage, fetchImage } from "../../api/device";

function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(" ");
}

// --- Card / Section --------------------------------------------------------

export function Card({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cx(
        "rounded-2xl border border-border bg-surface shadow-sm",
        className,
      )}
    >
      {children}
    </div>
  );
}

export function SectionHeader({
  icon: Icon,
  title,
  actions,
}: {
  icon?: LucideIcon;
  title: string;
  actions?: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        {Icon && <Icon size={18} className="text-muted" aria-hidden />}
        <h2 className="text-base font-bold tracking-tight text-fg">{title}</h2>
      </div>
      {actions}
    </div>
  );
}

// --- Button ----------------------------------------------------------------

type Variant = "primary" | "secondary" | "ghost" | "danger";

const VARIANTS: Record<Variant, string> = {
  primary:
    "bg-primary text-primary-fg hover:opacity-90 disabled:opacity-40",
  secondary:
    "bg-surface-2 text-fg hover:bg-border disabled:opacity-40",
  ghost:
    "bg-transparent text-fg hover:bg-surface-2 disabled:opacity-40",
  danger:
    "bg-danger text-danger-fg hover:opacity-90 disabled:opacity-40",
};

export function Button({
  variant = "secondary",
  size = "md",
  pill = false,
  loading = false,
  icon: Icon,
  children,
  className,
  disabled,
  ...rest
}: {
  variant?: Variant;
  size?: "sm" | "md";
  pill?: boolean;
  loading?: boolean;
  icon?: LucideIcon;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  const iconSize = size === "sm" ? 14 : 16;
  return (
    <button
      {...rest}
      disabled={disabled || loading}
      className={cx(
        "relative inline-flex cursor-pointer items-center justify-center font-semibold transition-colors duration-150",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-bg",
        "disabled:cursor-not-allowed",
        size === "sm" ? "px-3 py-1.5 text-xs" : "px-4 py-2 text-sm",
        pill ? "rounded-full" : "rounded-xl",
        VARIANTS[variant],
        className,
      )}
    >
      {/* Spinner overlays the (invisible) label so the button keeps its exact
          width when loading — no growth or smear. */}
      {loading && (
        <span className="absolute inset-0 flex items-center justify-center">
          <Loader2 size={iconSize} className="animate-spin" aria-hidden />
        </span>
      )}
      <span className={cx("inline-flex items-center gap-2", loading && "invisible")}>
        {Icon && <Icon size={iconSize} aria-hidden />}
        {children}
      </span>
    </button>
  );
}

export function IconButton({
  icon: Icon,
  label,
  variant = "secondary",
  size = "md",
  loading = false,
  className,
  disabled,
  ...rest
}: {
  icon: LucideIcon;
  label: string;
  variant?: Variant;
  size?: "sm" | "md";
  loading?: boolean;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  const dim = size === "sm" ? "h-8 w-8" : "h-10 w-10";
  const iconSize = size === "sm" ? 16 : 18;
  return (
    <button
      {...rest}
      disabled={disabled || loading}
      aria-label={label}
      title={label}
      className={cx(
        "inline-flex cursor-pointer items-center justify-center rounded-full transition-colors duration-150",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-bg",
        "disabled:cursor-not-allowed disabled:opacity-40",
        dim,
        VARIANTS[variant],
        className,
      )}
    >
      {loading ? (
        <Loader2 size={iconSize} className="animate-spin" aria-hidden />
      ) : (
        <Icon size={iconSize} aria-hidden />
      )}
    </button>
  );
}

// --- PillTabs --------------------------------------------------------------

export function PillTabs<T extends string>({
  tabs,
  active,
  onChange,
}: {
  tabs: { id: T; label: string }[];
  active: T;
  onChange: (id: T) => void;
}) {
  return (
    // Full-width iOS-style segmented switch: an inset track with equal-width
    // segments; the active one is a raised pill.
    <div className="flex w-full gap-1 rounded-xl bg-surface-2 p-1">
      {tabs.map((t) => (
        <button
          key={t.id}
          type="button"
          onClick={() => onChange(t.id)}
          className={cx(
            "flex-1 cursor-pointer rounded-lg px-3 py-1.5 text-center text-sm font-semibold transition-all duration-150",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            active === t.id
              ? "bg-primary text-primary-fg shadow-sm"
              : "text-muted hover:text-fg",
          )}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}

// --- Form controls ---------------------------------------------------------

export function Field({
  label,
  htmlFor,
  hint,
  error,
  children,
}: {
  label: string;
  htmlFor?: string;
  hint?: string;
  error?: string;
  children: ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <label htmlFor={htmlFor} className="block text-sm font-medium text-fg">
        {label}
      </label>
      {children}
      {error ? (
        <p className="text-xs text-danger" role="alert">
          {error}
        </p>
      ) : hint ? (
        <p className="text-xs text-muted">{hint}</p>
      ) : null}
    </div>
  );
}

// Fixed h-10 so <Input> and native <Select> render the exact same height.
const CONTROL =
  "h-10 w-full rounded-xl border border-border bg-surface-2 px-3 text-sm text-fg placeholder:text-subtle " +
  "transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

export function Input({
  className,
  ...rest
}: InputHTMLAttributes<HTMLInputElement>) {
  return <input {...rest} className={cx(CONTROL, className)} />;
}

export function Select({
  className,
  children,
  ...rest
}: SelectHTMLAttributes<HTMLSelectElement>) {
  // `appearance-none` strips the native macOS select chrome (which ignores
  // border-radius), so it matches <Input>'s rounded-xl exactly; a custom
  // chevron replaces the native one.
  return (
    <div className="relative">
      <select
        {...rest}
        className={cx(CONTROL, "cursor-pointer appearance-none pr-9", className)}
      >
        {children}
      </select>
      <ChevronDown
        size={16}
        className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-muted"
        aria-hidden
      />
    </div>
  );
}

// --- Toggle ----------------------------------------------------------------

export function Toggle({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cx(
        "relative inline-flex h-7 w-12 shrink-0 cursor-pointer items-center rounded-full transition-colors duration-200",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-bg",
        "disabled:cursor-not-allowed disabled:opacity-40",
        checked ? "bg-accent" : "bg-surface-2 border border-border",
      )}
    >
      <span
        className={cx(
          "inline-block h-5 w-5 transform rounded-full bg-white shadow transition-transform duration-200",
          checked ? "translate-x-6" : "translate-x-1",
        )}
      />
    </button>
  );
}

// --- Status / Badge --------------------------------------------------------

const DOT_TONE = {
  online: "bg-accent",
  offline: "bg-danger",
  idle: "bg-subtle",
} as const;

export function StatusDot({ tone }: { tone: keyof typeof DOT_TONE }) {
  return (
    <span
      className={cx("inline-block h-2.5 w-2.5 rounded-full", DOT_TONE[tone])}
      aria-hidden
    />
  );
}

const BADGE_TONE = {
  neutral: "bg-surface-2 text-muted",
  online: "bg-accent/15 text-accent-strong",
  danger: "bg-danger/15 text-danger",
} as const;

export function Badge({
  tone = "neutral",
  children,
}: {
  tone?: keyof typeof BADGE_TONE;
  children: ReactNode;
}) {
  return (
    <span
      className={cx(
        "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-semibold",
        BADGE_TONE[tone],
      )}
    >
      {children}
    </span>
  );
}

// --- ListRow ---------------------------------------------------------------

export function ListRow({
  icon: Icon,
  title,
  subtitle,
  right,
  onClick,
}: {
  icon?: LucideIcon;
  title: ReactNode;
  subtitle?: ReactNode;
  right?: ReactNode;
  onClick?: () => void;
}) {
  const clickable = Boolean(onClick);
  return (
    <div
      onClick={onClick}
      role={clickable ? "button" : undefined}
      tabIndex={clickable ? 0 : undefined}
      onKeyDown={
        clickable
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onClick?.();
              }
            }
          : undefined
      }
      className={cx(
        "flex items-center gap-3 px-4 py-3",
        clickable &&
          "cursor-pointer transition-colors hover:bg-surface-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
      )}
    >
      {Icon && (
        <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-surface-2 text-fg">
          <Icon size={20} aria-hidden />
        </span>
      )}
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-semibold text-fg">{title}</div>
        {subtitle && (
          <div className="truncate text-xs text-muted">{subtitle}</div>
        )}
      </div>
      {right ?? (clickable && <ChevronRight size={18} className="text-subtle" aria-hidden />)}
    </div>
  );
}

// --- EmptyState ------------------------------------------------------------

export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
}: {
  icon: LucideIcon;
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 px-6 py-12 text-center">
      <span className="flex h-12 w-12 items-center justify-center rounded-2xl bg-surface-2 text-muted">
        <Icon size={24} aria-hidden />
      </span>
      <div>
        <p className="text-sm font-bold text-fg">{title}</p>
        {description && (
          <p className="mx-auto mt-1 max-w-xs text-xs text-muted">{description}</p>
        )}
      </div>
      {action}
    </div>
  );
}

// --- DeviceImage -----------------------------------------------------------

/**
 * Lazily loads a device image (via `fetchImage`, cached in-memory + on disk)
 * and renders it, with a spinner while loading and an icon fallback on error.
 * One full-res copy is cached per image; size it down with `className`.
 */
export function DeviceImage({
  gallery,
  name,
  className,
  imgClassName,
  alt,
}: {
  gallery: string;
  name: string;
  className?: string;
  imgClassName?: string;
  alt?: string;
}) {
  // Seed synchronously from the session cache so a re-mount paints instantly
  // (no spinner flash) instead of waiting a microtask.
  const [src, setSrc] = useState<string | null>(() => cachedImage(gallery, name) ?? null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const cached = cachedImage(gallery, name);
    if (cached) {
      setSrc(cached);
      setFailed(false);
      return;
    }
    let alive = true;
    setSrc(null);
    setFailed(false);
    fetchImage(gallery, name)
      .then((url) => alive && setSrc(url))
      .catch(() => alive && setFailed(true));
    return () => {
      alive = false;
    };
  }, [gallery, name]);

  return (
    <div className={cx("relative overflow-hidden bg-surface-2", className)}>
      {src && (
        <img
          src={src}
          alt={alt ?? name}
          // Promote the image onto its own compositing layer so a sibling's
          // hover repaint (the fade-in Show/Delete overlay) doesn't force the
          // rounded-corner clip to re-rasterize — that re-raster is what makes
          // the thumbnail shimmer/jitter on hover in WKWebView.
          className={cx(
            "h-full w-full object-cover [backface-visibility:hidden] [transform:translateZ(0)]",
            imgClassName,
          )}
        />
      )}
      {!src && !failed && (
        <div className="absolute inset-0 flex items-center justify-center">
          <Spinner size={20} />
        </div>
      )}
      {failed && (
        <div className="absolute inset-0 flex items-center justify-center text-subtle">
          <ImageOff size={20} aria-hidden />
        </div>
      )}
    </div>
  );
}

// --- Spinner ---------------------------------------------------------------

export function Spinner({ size = 20 }: { size?: number }) {
  return <Loader2 size={size} className="animate-spin text-muted" aria-label="Loading" />;
}

// --- Toast -----------------------------------------------------------------

type Toast = { id: number; tone: "success" | "error"; message: string };
type ToastCtx = { show: (tone: Toast["tone"], message: string) => void };

const ToastContext = createContext<ToastCtx | null>(null);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);

  const show = useCallback((tone: Toast["tone"], message: string) => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t, { id, tone, message }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 3500);
  }, []);

  return (
    <ToastContext.Provider value={{ show }}>
      {children}
      <div
        className="pointer-events-none fixed bottom-4 right-4 z-50 flex flex-col gap-2"
        aria-live="polite"
      >
        {toasts.map((t) => {
          const Icon = t.tone === "success" ? CheckCircle2 : AlertCircle;
          return (
            <div
              key={t.id}
              role="status"
              className={cx(
                "pointer-events-auto flex max-w-xs items-start gap-2.5 rounded-xl border px-4 py-3 text-sm font-medium shadow-lg",
                "border-border bg-surface text-fg",
                t.tone === "success" ? "border-l-4 border-l-accent" : "border-l-4 border-l-danger",
              )}
            >
              <Icon
                size={18}
                className={cx(
                  "mt-0.5 shrink-0",
                  t.tone === "success" ? "text-accent-strong" : "text-danger",
                )}
                aria-hidden
              />
              <span className="[overflow-wrap:anywhere]">{t.message}</span>
            </div>
          );
        })}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast(): ToastCtx {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within ToastProvider");
  return ctx;
}

export { cx };

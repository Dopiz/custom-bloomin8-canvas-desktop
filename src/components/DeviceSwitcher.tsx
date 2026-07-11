import { useEffect, useRef, useState } from "react";
import {
  BluetoothSearching,
  Check,
  CheckCircle2,
  ChevronsUpDown,
  MonitorSmartphone,
  Plus,
  Trash2,
} from "lucide-react";
import type { DeviceEntry } from "../types";
import { bleScan, errorMessage } from "../api/device";
import ConfirmDialog from "./ConfirmDialog";
import { Button, Field, Input, cx } from "./ui";

/** Data collected by the add-device dialog: the LAN IP, the user's BLE match
 * hint, and the real device name resolved from a BLE scan (or the hint/IP when
 * no device answered). */
export interface AddDeviceData {
  lanIp: string;
  bleHint: string;
  name: string;
}

interface DeviceSwitcherProps {
  devices: DeviceEntry[];
  /** Effective active device id (falls back to first device), or null. */
  activeId: string | null;
  /** The active device's live settings name (from `/deviceInfo`), resolved by
   * the parent so it stays in sync with the Device page hero even after a
   * rename. `null` when unknown/unreachable — falls back to the config name. */
  liveName: string | null;
  onSwitch: (id: string) => void | Promise<void>;
  onDelete: (id: string) => void | Promise<void>;
  onAdd: (data: AddDeviceData) => void | Promise<void>;
}

/** Sidebar device picker: collapsed shows the active device's name; expanded
 * lists every configured device (active one highlighted + checked), each with
 * a delete button, plus an "Add new device" entry that opens a small dialog. */
export default function DeviceSwitcher({
  devices,
  activeId,
  liveName,
  onSwitch,
  onDelete,
  onAdd,
}: DeviceSwitcherProps) {
  const [open, setOpen] = useState(false);
  const [adding, setAdding] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<DeviceEntry | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const active = devices.find((d) => d.id === activeId) ?? null;

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={containerRef} className="relative px-1 pt-4">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="listbox"
        aria-expanded={open}
        className={cx(
          "flex w-full cursor-pointer items-center gap-2 rounded-xl border border-border bg-surface-2 px-3 py-2 text-left transition-colors",
          "hover:bg-border focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        )}
      >
        <MonitorSmartphone size={16} className="shrink-0 text-muted" aria-hidden />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-fg">
          {liveName || active?.name || active?.lan_ip || "No device"}
        </span>
        <ChevronsUpDown size={15} className="shrink-0 text-subtle" aria-hidden />
      </button>

      {open && (
        <div
          role="listbox"
          className="absolute bottom-full left-0 right-0 mb-1 overflow-hidden rounded-xl border border-border bg-surface p-1 shadow-lg"
        >
          {devices.length === 0 && (
            <p className="px-3 py-2 text-xs text-muted">No devices yet.</p>
          )}
          {devices.map((d) => {
            const isActive = d.id === activeId;
            return (
              <div
                key={d.id}
                className={cx(
                  "group flex items-center gap-1 rounded-lg",
                  isActive ? "bg-surface-2" : "hover:bg-surface-2",
                )}
              >
                <button
                  type="button"
                  role="option"
                  aria-selected={isActive}
                  onClick={() => {
                    setOpen(false);
                    if (!isActive) void onSwitch(d.id);
                  }}
                  className="flex min-w-0 flex-1 cursor-pointer items-center gap-2 rounded-lg px-2.5 py-2 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <Check
                    size={15}
                    className={cx("shrink-0", isActive ? "text-accent-strong" : "text-transparent")}
                    aria-hidden
                  />
                  <span className="min-w-0 flex-1 truncate text-sm font-medium text-fg">
                    {(isActive && liveName) || d.name || d.lan_ip}
                  </span>
                </button>
                <button
                  type="button"
                  onClick={() => setPendingDelete(d)}
                  aria-label={`Delete ${d.name || d.lan_ip}`}
                  title="Delete device"
                  className="mr-1 flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-lg text-subtle transition-colors hover:bg-danger/15 hover:text-danger focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <Trash2 size={15} aria-hidden />
                </button>
              </div>
            );
          })}

          <div className="my-1 h-px bg-border" />

          <button
            type="button"
            onClick={() => {
              setOpen(false);
              setAdding(true);
            }}
            className="flex w-full cursor-pointer items-center gap-2 rounded-lg px-2.5 py-2 text-left text-sm font-medium text-fg transition-colors hover:bg-surface-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <Plus size={15} className="shrink-0 text-muted" aria-hidden />
            Add new device
          </button>
        </div>
      )}

      {adding && (
        <AddDeviceDialog
          existingLanIps={devices.map((d) => d.lan_ip.trim())}
          onCancel={() => setAdding(false)}
          onSubmit={async (data) => {
            await onAdd(data);
            setAdding(false);
          }}
        />
      )}

      {pendingDelete && (
        <ConfirmDialog
          title="Delete device?"
          message={`Remove "${pendingDelete.name || pendingDelete.lan_ip}" from this app. This doesn't touch the device itself.`}
          confirmLabel="Delete"
          onCancel={() => setPendingDelete(null)}
          onConfirm={() => {
            const id = pendingDelete.id;
            setPendingDelete(null);
            void onDelete(id);
          }}
        />
      )}
    </div>
  );
}

/** Small modal for adding a device: its LAN IP plus an optional BLE match hint
 * ("Device name"). On submit it scans BLE for the hint to confirm the Canvas is
 * reachable — a match auto-saves under the real advertised name; no match shows
 * a warning but still lets the user save (the device may just be asleep). */
export function AddDeviceDialog({
  existingLanIps,
  onCancel,
  onSubmit,
}: {
  existingLanIps: string[];
  onCancel: () => void;
  onSubmit: (data: AddDeviceData) => Promise<void>;
}) {
  const [lanIp, setLanIp] = useState("");
  const [bleHint, setBleHint] = useState("");
  const [error, setError] = useState("");
  const [scanning, setScanning] = useState(false);
  const [saving, setSaving] = useState(false);
  // Result of the last BLE scan for the current inputs. `null` means "not
  // scanned yet" — any input change clears it so the next submit re-scans.
  const [scanResult, setScanResult] = useState<{ found: boolean; name: string } | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  async function finishSave(name: string) {
    setSaving(true);
    try {
      await onSubmit({ lanIp: lanIp.trim(), bleHint: bleHint.trim(), name });
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }

  async function runScan() {
    setScanning(true);
    setError("");
    const effHint = bleHint.trim() || "Bloomin8";
    try {
      const matches = await bleScan(effHint);
      if (matches.length > 0) {
        const name = matches[0].name;
        setScanResult({ found: true, name });
        // Found it — save straight away under the real advertised name.
        await finishSave(name);
      } else {
        // Nothing answered — let the user save anyway (device may be asleep).
        setScanResult({ found: false, name: "" });
      }
    } catch {
      // A BLE failure (no adapter / no permission) shouldn't block adding a
      // device; treat it like "no match" so the user can still save.
      setScanResult({ found: false, name: "" });
    } finally {
      setScanning(false);
    }
  }

  function submit(e: React.FormEvent) {
    e.preventDefault();
    const ip = lanIp.trim();
    if (!ip) {
      setError("LAN IP is required.");
      return;
    }
    if (existingLanIps.includes(ip)) {
      setError("A device with this LAN IP already exists.");
      return;
    }
    // Second click after a no-match scan: save anyway under the hint (or IP).
    if (scanResult && !scanResult.found) {
      void finishSave(bleHint.trim() || ip);
      return;
    }
    void runScan();
  }

  const busy = scanning || saving;
  const submitLabel = scanning
    ? "Searching…"
    : scanResult && !scanResult.found
      ? "Add anyway"
      : "Add device";

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onClick={onCancel}
      role="dialog"
      aria-modal="true"
    >
      <form
        onClick={(e) => e.stopPropagation()}
        onSubmit={submit}
        className="w-full max-w-sm rounded-2xl border border-border bg-surface p-5 shadow-2xl"
      >
        <h3 className="text-base font-bold text-fg">Add device</h3>
        <div className="mt-4 space-y-4">
          <Field label="LAN IP" htmlFor="add-device-ip" error={error || undefined}>
            <Input
              autoFocus
              id="add-device-ip"
              value={lanIp}
              onChange={(e) => {
                setLanIp(e.currentTarget.value);
                if (error) setError("");
                if (scanResult) setScanResult(null);
              }}
              placeholder="192.168.0.100"
            />
          </Field>
          <Field
            label="Device name"
            htmlFor="add-device-name"
            hint='Used to find your Canvas over Bluetooth to wake it. Leave blank to use "Bloomin8".'
          >
            <Input
              id="add-device-name"
              value={bleHint}
              onChange={(e) => {
                setBleHint(e.currentTarget.value);
                if (scanResult) setScanResult(null);
              }}
              placeholder="Bloomin8"
            />
          </Field>

          {scanResult?.found && (
            <p className="flex items-center gap-1.5 text-xs font-medium text-accent-strong">
              <CheckCircle2 size={14} aria-hidden />
              Found “{scanResult.name}”
            </p>
          )}
          {scanResult && !scanResult.found && (
            <p className="flex items-start gap-1.5 text-xs text-muted">
              <BluetoothSearching size={14} className="mt-0.5 shrink-0" aria-hidden />
              No matching Bluetooth device found — it may be asleep or out of range.
            </p>
          )}
        </div>
        <div className="mt-5 flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" loading={busy}>
            {submitLabel}
          </Button>
        </div>
      </form>
    </div>
  );
}

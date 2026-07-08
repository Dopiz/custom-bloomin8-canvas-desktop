import { useEffect, useRef, useState } from "react";
import { ImagePlus, Loader2, MonitorSmartphone, Send, Trash2, Upload } from "lucide-react";
import {
  errorMessage,
  libraryAdd,
  libraryDelete,
  libraryImage,
  libraryList,
  type LibraryItem,
} from "../api/device";
import ConfirmDialog from "./ConfirmDialog";
import OnDeviceDialog from "./OnDeviceDialog";
import PushImageDialog from "./PushImageDialog";
import { Button, EmptyState, Spinner } from "./ui";

type LoadState = "loading" | "loaded" | "error" | "empty";

function fileToDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error ?? new Error("Failed to read file"));
    reader.readAsDataURL(file);
  });
}

/**
 * Gallery = the user's local image library (originals, stored on this
 * machine). Uploading never auto-pushes to the device — each image is
 * pushed individually via PushImageDialog, which lets the user choose
 * display settings first. What's actually on the device (and playlists,
 * a device-only concept) lives behind the "On device" button.
 */
export default function GalleryPage() {
  const [items, setItems] = useState<LibraryItem[]>([]);
  const [state, setState] = useState<LoadState>("loading");
  const [error, setError] = useState("");
  const [uploading, setUploading] = useState(false);
  const [pushTargetId, setPushTargetId] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [onDeviceOpen, setOnDeviceOpen] = useState(false);

  const fileInputRef = useRef<HTMLInputElement>(null);
  // In-memory cache of resolved original images (data URLs) keyed by library
  // id, shared across grid cells for this session — avoids re-invoking Tauri
  // for images already fetched.
  const imageCacheRef = useRef<Map<string, string>>(new Map());

  useEffect(() => {
    void loadLibrary();
  }, []);

  async function loadLibrary() {
    setState("loading");
    setError("");
    try {
      const list = await libraryList();
      setItems(list);
      setState(list.length > 0 ? "loaded" : "empty");
    } catch (e) {
      setState("error");
      setError(errorMessage(e));
    }
  }

  async function handleFiles(fileList: FileList | null) {
    if (!fileList || fileList.length === 0) return;
    setUploading(true);
    setError("");
    try {
      for (const file of Array.from(fileList)) {
        const dataUrl = await fileToDataUrl(file);
        await libraryAdd(file.name, dataUrl);
      }
      await loadLibrary();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setUploading(false);
      if (fileInputRef.current) fileInputRef.current.value = "";
    }
  }

  async function handleDelete(id: string) {
    setConfirmDeleteId(null);
    setDeletingId(id);
    setError("");
    try {
      await libraryDelete(id);
      imageCacheRef.current.delete(id);
      await loadLibrary();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setDeletingId(null);
    }
  }

  return (
    <div className="mx-auto max-w-3xl space-y-5 p-6">
      <div className="flex items-center justify-between gap-3">
        <h1 className="text-3xl font-extrabold tracking-tight text-fg">Gallery</h1>
        <div className="flex items-center gap-2">
          <Button
            variant="secondary"
            icon={MonitorSmartphone}
            onClick={() => setOnDeviceOpen(true)}
          >
            On device
          </Button>
          <Button
            variant="primary"
            icon={Upload}
            loading={uploading}
            onClick={() => fileInputRef.current?.click()}
          >
            Upload
          </Button>
        </div>
      </div>

      <input
        ref={fileInputRef}
        type="file"
        accept="image/*"
        multiple
        className="hidden"
        onChange={(e) => void handleFiles(e.currentTarget.files)}
      />

      {error && <p className="text-sm text-danger">{error}</p>}

      {state === "loading" && items.length === 0 && (
        <div className="flex items-center justify-center py-16">
          <Spinner />
        </div>
      )}

      {state === "error" && items.length === 0 && (
        <div className="space-y-2">
          <Button size="sm" onClick={() => void loadLibrary()}>
            Retry
          </Button>
        </div>
      )}

      {state === "empty" && (
        <EmptyState
          icon={ImagePlus}
          title="No images yet"
          description="Upload an image to get started."
          action={
            <Button
              variant="primary"
              icon={Upload}
              onClick={() => fileInputRef.current?.click()}
            >
              Upload
            </Button>
          }
        />
      )}

      {items.length > 0 && (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
          {items.map((item) => (
            <LibraryCell
              key={item.id}
              item={item}
              cache={imageCacheRef.current}
              busy={deletingId === item.id}
              onPush={() => setPushTargetId(item.id)}
              onDelete={() => setConfirmDeleteId(item.id)}
            />
          ))}
        </div>
      )}

      {pushTargetId && (
        <PushImageDialog libraryId={pushTargetId} onClose={() => setPushTargetId(null)} />
      )}

      {onDeviceOpen && <OnDeviceDialog onClose={() => setOnDeviceOpen(false)} />}

      {confirmDeleteId !== null && (
        <ConfirmDialog
          title="Delete this image?"
          message="This removes the original from your local library. It doesn't affect anything already pushed to the device."
          confirmLabel="Delete"
          onCancel={() => setConfirmDeleteId(null)}
          onConfirm={() => void handleDelete(confirmDeleteId)}
        />
      )}
    </div>
  );
}

function LibraryCell({
  item,
  cache,
  busy,
  onPush,
  onDelete,
}: {
  item: LibraryItem;
  cache: Map<string, string>;
  busy: boolean;
  onPush: () => void;
  onDelete: () => void;
}) {
  const [src, setSrc] = useState<string | null>(() => cache.get(item.id) ?? null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const cached = cache.get(item.id);
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
        cache.set(item.id, url);
        if (alive) setSrc(url);
      })
      .catch(() => {
        if (alive) setFailed(true);
      });
    return () => {
      alive = false;
    };
  }, [item.id, cache]);

  return (
    <div className="group relative aspect-square overflow-hidden rounded-xl bg-surface-2">
      {src && <img src={src} alt={item.name} className="h-full w-full object-cover" />}
      {!src && !failed && (
        <div className="absolute inset-0 flex items-center justify-center">
          <Spinner size={20} />
        </div>
      )}
      {failed && (
        <div className="absolute inset-0 flex items-center justify-center text-subtle">
          <ImagePlus size={20} aria-hidden />
        </div>
      )}
      <div className="absolute bottom-2 right-2 flex gap-1.5 opacity-0 transition-opacity duration-150 group-hover:opacity-100">
        <button
          type="button"
          title="Push to device"
          aria-label="Push to device"
          onClick={onPush}
          className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full bg-black/50 text-white transition-colors hover:bg-black/70 focus:outline-none focus-visible:ring-2 focus-visible:ring-white/70"
        >
          <Send size={15} aria-hidden />
        </button>
        <button
          type="button"
          title="Delete"
          aria-label="Delete"
          disabled={busy}
          onClick={onDelete}
          className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full bg-black/50 text-white transition-colors hover:bg-black/70 focus:outline-none focus-visible:ring-2 focus-visible:ring-white/70 disabled:cursor-not-allowed disabled:opacity-60"
        >
          {busy ? (
            <Loader2 size={15} className="animate-spin" aria-hidden />
          ) : (
            <Trash2 size={15} aria-hidden />
          )}
        </button>
      </div>
    </div>
  );
}

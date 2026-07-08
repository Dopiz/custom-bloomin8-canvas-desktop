import { useEffect, useState } from "react";
import {
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  Images,
  ListMusic,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  Trash2,
  X,
} from "lucide-react";
import {
  errorMessage,
  galleryImages,
  imageDelete,
  playlistDelete,
  playlistList,
  playlistSave,
  showImage,
  showPlaylist,
} from "../api/device";
import type { GalleryImage, PlaylistSummary } from "../types";
import ConfirmDialog from "./ConfirmDialog";
import {
  Button,
  DeviceImage,
  EmptyState,
  Field,
  IconButton,
  Input,
  ListRow,
  SectionHeader,
  Spinner,
  useToast,
} from "./ui";

type LoadState = "loading" | "loaded" | "error" | "empty";

const IMAGES_PER_PAGE = 10;

/** The device only ever exposes a single gallery to this app. */
const GALLERY = "default";

/** Playlists are temporarily disabled for the first release (the flow needs
 * more work). All the code is kept behind this flag so it can be re-enabled by
 * flipping it back to `true`. See README. */
const PLAYLISTS_ENABLED = false;

/** Used only to pull the gallery's full image list for the playlist editor's
 * picker — comfortably above what the gallery is expected to hold. */
const PLAYLIST_PICKER_LIMIT = 200;

/**
 * "On device" viewer: shows what images actually currently exist on the
 * device (the raw originals the device returns, view-only oriented besides
 * show/delete), and hosts the playlist editor — playlists are a device
 * concept, so they live here rather than in the local library grid.
 */
export default function OnDeviceDialog({ onClose }: { onClose: () => void }) {
  // Images (paginated)
  const [images, setImages] = useState<GalleryImage[]>([]);
  const [imagesState, setImagesState] = useState<LoadState>("loading");
  const [imagesError, setImagesError] = useState("");
  const [offset, setOffset] = useState(0);
  const [hasNextPage, setHasNextPage] = useState(false);
  const [rowBusy, setRowBusy] = useState<{ name: string; action: "show" | "delete" } | null>(
    null,
  );
  const [confirmDeleteImage, setConfirmDeleteImage] = useState<string | null>(null);


  // Playlists
  const [playlists, setPlaylists] = useState<PlaylistSummary[]>([]);
  const [playlistsState, setPlaylistsState] = useState<LoadState>("loading");
  const [playlistsError, setPlaylistsError] = useState("");
  const [editingPlaylist, setEditingPlaylist] = useState<string | null>(null);
  const [playlistNameInput, setPlaylistNameInput] = useState("");
  const [playlistItems, setPlaylistItems] = useState<string[]>([]);
  // Seconds each image is shown before advancing (playlist `type: "duration"`).
  const [playlistSeconds, setPlaylistSeconds] = useState("30");
  const [pickerImages, setPickerImages] = useState<GalleryImage[]>([]);
  const [pickerState, setPickerState] = useState<LoadState>("empty");
  const [playlistSaveState, setPlaylistSaveState] = useState<"idle" | "saving" | "saved" | "error">(
    "idle",
  );
  const [playlistRowBusy, setPlaylistRowBusy] = useState<string | null>(null);
  const [confirmDeletePlaylist, setConfirmDeletePlaylist] = useState<string | null>(null);
  const toast = useToast();

  useEffect(() => {
    void loadImages(GALLERY, 0);
    void refreshPlaylists();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function loadImages(gallery: string, nextOffset: number) {
    setImagesState("loading");
    setImagesError("");
    try {
      const page = await galleryImages(gallery, nextOffset, IMAGES_PER_PAGE);
      setImages(page);
      setOffset(nextOffset);
      setHasNextPage(page.length === IMAGES_PER_PAGE);
      setImagesState(page.length > 0 || nextOffset > 0 ? "loaded" : "empty");
    } catch (e) {
      setImagesState("error");
      setImagesError(errorMessage(e));
    }
  }

  function refreshAll() {
    void loadImages(GALLERY, offset);
    void refreshPlaylists();
  }

  async function handleShowNow(name: string) {
    setRowBusy({ name, action: "show" });
    try {
      await showImage(GALLERY, name);
    } catch (e) {
      toast.show("error", errorMessage(e));
    } finally {
      setRowBusy(null);
    }
  }

  async function handleDeleteImage(name: string) {
    setConfirmDeleteImage(null);
    setRowBusy({ name, action: "delete" });
    try {
      await imageDelete(GALLERY, name);
      await loadImages(GALLERY, offset);
    } catch (e) {
      toast.show("error", errorMessage(e));
    } finally {
      setRowBusy(null);
    }
  }

  async function refreshPlaylists() {
    setPlaylistsState("loading");
    setPlaylistsError("");
    try {
      const result = await playlistList();
      setPlaylists(result);
      setPlaylistsState("loaded");
    } catch (e) {
      setPlaylistsState("error");
      setPlaylistsError(errorMessage(e));
    }
  }

  async function openPlaylistEditor(name: string | null) {
    setEditingPlaylist(name ?? "__new__");
    setPlaylistNameInput(name ?? "");
    setPlaylistItems([]);
    setPlaylistSeconds("30");
    setPlaylistSaveState("idle");

    setPickerState("loading");
    try {
      const all = await galleryImages(GALLERY, 0, PLAYLIST_PICKER_LIMIT);
      setPickerImages(all);
      setPickerState(all.length > 0 ? "loaded" : "empty");
    } catch {
      setPickerState("error");
      setPickerImages([]);
    }
  }

  function closePlaylistEditor() {
    setEditingPlaylist(null);
  }

  function addToPlaylist(name: string) {
    setPlaylistItems((prev) => [...prev, name]);
  }

  function removeFromPlaylist(index: number) {
    setPlaylistItems((prev) => prev.filter((_, i) => i !== index));
  }

  function moveItem(index: number, direction: -1 | 1) {
    setPlaylistItems((prev) => {
      const next = [...prev];
      const target = index + direction;
      if (target < 0 || target >= next.length) return prev;
      [next[index], next[target]] = [next[target], next[index]];
      return next;
    });
  }

  async function savePlaylist(e: React.FormEvent) {
    e.preventDefault();
    const name = playlistNameInput.trim();
    if (!name) return;
    const seconds = Math.max(1, Math.round(Number(playlistSeconds) || 30));
    setPlaylistSaveState("saving");
    try {
      await playlistSave(name, {
        name,
        type: "duration",
        list: playlistItems.map((img) => ({
          name: `/gallerys/${GALLERY}/${img}`,
          duration: seconds,
          time: "",
        })),
      });
      setPlaylistSaveState("saved");
      toast.show("success", "Playlist saved");
      await refreshPlaylists();
      setEditingPlaylist(null);
    } catch (e) {
      setPlaylistSaveState("error");
      toast.show("error", errorMessage(e));
    }
  }

  async function handlePlayPlaylist(name: string) {
    setPlaylistRowBusy(name);
    try {
      await showPlaylist(name);
      toast.show("success", `Playing "${name}"`);
    } catch (e) {
      toast.show("error", errorMessage(e));
    } finally {
      setPlaylistRowBusy(null);
    }
  }

  async function handleDeletePlaylist(name: string) {
    setConfirmDeletePlaylist(null);
    setPlaylistRowBusy(name);
    try {
      await playlistDelete(name);
      await refreshPlaylists();
    } catch (e) {
      toast.show("error", errorMessage(e));
    } finally {
      setPlaylistRowBusy(null);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
    >
      <div
        className="max-h-[90vh] w-full max-w-3xl overflow-y-auto rounded-2xl border border-border bg-surface p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between gap-3">
          <h2 className="text-xl font-extrabold tracking-tight text-fg">On device</h2>
          <div className="flex items-center gap-2">
            <IconButton
              icon={RefreshCw}
              label="Refresh"
              onClick={refreshAll}
              disabled={imagesState === "loading" || playlistsState === "loading"}
            />
            <IconButton icon={X} label="Close" onClick={onClose} />
          </div>
        </div>

        {/* Device images */}
        <div className="mt-5">
          <SectionHeader icon={Images} title="Images on device" />

          <div className="mt-4">
            {imagesState === "error" && (
              <div className="space-y-2">
                <p className="text-sm text-danger">{imagesError}</p>
                <Button size="sm" onClick={() => void loadImages(GALLERY, offset)}>
                  Retry
                </Button>
              </div>
            )}
            {imagesState === "empty" && (
              <EmptyState
                icon={Images}
                title="No images yet"
                description="No images have been pushed to the device."
              />
            )}

            {imagesState === "loading" && images.length === 0 && (
              <div className="flex items-center justify-center py-6">
                <Spinner />
              </div>
            )}

            {images.length > 0 && (
              <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
                {images.map((img) => (
                  <div key={img.name} className="group relative aspect-square overflow-hidden rounded-xl">
                    <DeviceImage
                      gallery={GALLERY}
                      name={img.name}
                      className="h-full w-full"
                      imgClassName="object-cover"
                    />
                    <div className="absolute bottom-2 right-2 flex gap-1.5 opacity-0 transition-opacity duration-150 group-hover:opacity-100">
                      <button
                        type="button"
                        title="Show now"
                        aria-label="Show now"
                        disabled={rowBusy?.name === img.name}
                        onClick={() => void handleShowNow(img.name)}
                        className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full bg-black/50 text-white transition-colors hover:bg-black/70 focus:outline-none focus-visible:ring-2 focus-visible:ring-white/70 disabled:cursor-not-allowed disabled:opacity-60"
                      >
                        {rowBusy?.name === img.name && rowBusy.action === "show" ? (
                          <Spinner size={15} />
                        ) : (
                          <Play size={15} aria-hidden />
                        )}
                      </button>
                      <button
                        type="button"
                        title="Delete"
                        aria-label="Delete"
                        disabled={rowBusy?.name === img.name}
                        onClick={() => setConfirmDeleteImage(img.name)}
                        className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full bg-black/50 text-white transition-colors hover:bg-black/70 focus:outline-none focus-visible:ring-2 focus-visible:ring-white/70 disabled:cursor-not-allowed disabled:opacity-60"
                      >
                        {rowBusy?.name === img.name && rowBusy.action === "delete" ? (
                          <Spinner size={15} />
                        ) : (
                          <Trash2 size={15} aria-hidden />
                        )}
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}

            <div className="mt-4 flex items-center justify-center gap-2">
              <IconButton
                icon={ChevronLeft}
                label="Previous page"
                size="sm"
                disabled={offset === 0 || imagesState === "loading"}
                onClick={() => void loadImages(GALLERY, Math.max(0, offset - IMAGES_PER_PAGE))}
              />
              <span className="tabular text-xs text-muted">
                Page {Math.floor(offset / IMAGES_PER_PAGE) + 1}
              </span>
              <IconButton
                icon={ChevronRight}
                label="Next page"
                size="sm"
                disabled={!hasNextPage || imagesState === "loading"}
                onClick={() => void loadImages(GALLERY, offset + IMAGES_PER_PAGE)}
              />
            </div>
          </div>
        </div>

        {/* Playlists — temporarily disabled for the first release (see README). */}
        {PLAYLISTS_ENABLED && (
        <div className="mt-6 border-t border-border pt-5">
          <SectionHeader
            icon={ListMusic}
            title="Playlists"
            actions={
              <Button size="sm" icon={Plus} onClick={() => void openPlaylistEditor(null)}>
                New playlist
              </Button>
            }
          />

          <div className="mt-4 space-y-4">
            {editingPlaylist !== null && (
              <form
                onSubmit={savePlaylist}
                className="space-y-4 rounded-xl border border-border bg-surface-2/40 p-4"
              >
                <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
                  <Field label="Playlist name" htmlFor="playlist-name">
                    <Input
                      id="playlist-name"
                      value={playlistNameInput}
                      onChange={(e) => setPlaylistNameInput(e.currentTarget.value)}
                      disabled={editingPlaylist !== "__new__"}
                    />
                  </Field>
                  <Field
                    label="Seconds per image"
                    htmlFor="playlist-seconds"
                    hint="How long each image shows before advancing."
                  >
                    <Input
                      id="playlist-seconds"
                      type="number"
                      min={1}
                      value={playlistSeconds}
                      onChange={(e) => setPlaylistSeconds(e.currentTarget.value)}
                    />
                  </Field>
                </div>

                <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
                  <div>
                    <p className="mb-1.5 text-sm font-medium text-fg">Available images</p>
                    {pickerState === "loading" && (
                      <div className="flex items-center justify-center py-4">
                        <Spinner size={16} />
                      </div>
                    )}
                    {pickerState === "error" && (
                      <p className="text-sm text-danger">Could not load images for this gallery.</p>
                    )}
                    {pickerState === "empty" && (
                      <p className="text-sm text-muted">No images available.</p>
                    )}
                    <div className="grid max-h-64 grid-cols-5 gap-2 overflow-y-auto">
                      {pickerImages.map((img) => (
                        <button
                          key={img.name}
                          type="button"
                          title={img.name}
                          onClick={() => addToPlaylist(img.name)}
                          className="group relative cursor-pointer rounded-lg focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                        >
                          <DeviceImage
                            gallery={GALLERY}
                            name={img.name}
                            className="aspect-square w-full rounded-lg border border-border"
                            imgClassName="object-cover"
                          />
                          <span className="absolute bottom-0.5 right-0.5 flex h-5 w-5 items-center justify-center rounded-full bg-primary text-primary-fg">
                            <Plus size={12} />
                          </span>
                        </button>
                      ))}
                    </div>
                  </div>

                  <div>
                    <p className="mb-1.5 text-sm font-medium text-fg">
                      Playlist order ({playlistItems.length})
                    </p>
                    {playlistItems.length === 0 && (
                      <p className="text-sm text-muted">Add images from the left.</p>
                    )}
                    <div className="grid max-h-64 grid-cols-3 gap-2 overflow-y-auto">
                      {playlistItems.map((name, i) => (
                        <div key={`${name}-${i}`} className="group relative">
                          <DeviceImage
                            gallery={GALLERY}
                            name={name}
                            className="aspect-[3/4] w-full rounded-lg border border-border"
                            imgClassName="object-cover"
                          />
                          <span className="absolute left-1 top-1 rounded-full bg-black/70 px-1.5 py-0.5 text-xs font-bold text-white">
                            {i + 1}
                          </span>
                          <div className="absolute inset-x-0 bottom-0 flex justify-center gap-1 rounded-b-lg bg-black/55 p-1 opacity-0 transition-opacity group-hover:opacity-100">
                            <button
                              type="button"
                              title="Move up"
                              disabled={i === 0}
                              onClick={() => moveItem(i, -1)}
                              className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-full bg-white/20 text-white hover:bg-white/30 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:cursor-not-allowed disabled:opacity-40"
                            >
                              <ChevronUp size={14} />
                            </button>
                            <button
                              type="button"
                              title="Move down"
                              disabled={i === playlistItems.length - 1}
                              onClick={() => moveItem(i, 1)}
                              className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-full bg-white/20 text-white hover:bg-white/30 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:cursor-not-allowed disabled:opacity-40"
                            >
                              <ChevronDown size={14} />
                            </button>
                            <button
                              type="button"
                              title="Remove from playlist"
                              onClick={() => removeFromPlaylist(i)}
                              className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-full bg-white/20 text-white hover:bg-white/30 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent"
                            >
                              <X size={14} />
                            </button>
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                </div>

                <div className="flex items-center justify-end gap-3">
                  <Button
                    type="submit"
                    variant="primary"
                    loading={playlistSaveState === "saving"}
                    disabled={!playlistNameInput.trim()}
                  >
                    Save playlist
                  </Button>
                  <Button type="button" variant="ghost" onClick={closePlaylistEditor}>
                    Cancel
                  </Button>
                </div>
              </form>
            )}

            {playlistsState === "loading" && (
              <div className="flex items-center justify-center py-6">
                <Spinner />
              </div>
            )}
            {playlistsState === "error" && (
              <div className="space-y-2">
                <p className="text-sm text-danger">{playlistsError}</p>
                <Button size="sm" onClick={() => void refreshPlaylists()}>
                  Retry
                </Button>
              </div>
            )}
            {playlistsState === "loaded" && playlists.length === 0 && (
              <EmptyState icon={ListMusic} title="No playlists yet" description="Create one to get started." />
            )}

            {playlists.length > 0 && (
              <div className="-mx-5 divide-y divide-border">
                {playlists.map((p) => (
                  <ListRow
                    key={p.name}
                    icon={ListMusic}
                    title={p.name}
                    right={
                      <div className="flex shrink-0 gap-2">
                        <Button
                          size="sm"
                          icon={Play}
                          disabled={playlistRowBusy === p.name}
                          loading={playlistRowBusy === p.name}
                          onClick={() => void handlePlayPlaylist(p.name)}
                        >
                          Play
                        </Button>
                        <Button
                          size="sm"
                          icon={Pencil}
                          disabled={playlistRowBusy === p.name}
                          onClick={() => void openPlaylistEditor(p.name)}
                        >
                          Edit
                        </Button>
                        <Button
                          variant="danger"
                          size="sm"
                          icon={Trash2}
                          disabled={playlistRowBusy === p.name}
                          onClick={() => setConfirmDeletePlaylist(p.name)}
                        >
                          Delete
                        </Button>
                      </div>
                    }
                  />
                ))}
              </div>
            )}
          </div>
        </div>
        )}
      </div>

      {confirmDeleteImage !== null && (
        <ConfirmDialog
          title={`Delete "${confirmDeleteImage}"?`}
          message="This removes the image from the device."
          confirmLabel="Delete"
          onCancel={() => setConfirmDeleteImage(null)}
          onConfirm={() => void handleDeleteImage(confirmDeleteImage)}
        />
      )}
      {confirmDeletePlaylist !== null && (
        <ConfirmDialog
          title={`Delete playlist "${confirmDeletePlaylist}"?`}
          message="This removes the playlist from the device."
          confirmLabel="Delete"
          onCancel={() => setConfirmDeletePlaylist(null)}
          onConfirm={() => void handleDeletePlaylist(confirmDeletePlaylist)}
        />
      )}
    </div>
  );
}

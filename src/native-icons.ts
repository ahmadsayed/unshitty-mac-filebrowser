// Icons from the operating system (src-tauri/src/file_icons.rs): the ones
// Finder shows on macOS, the desktop's icon theme on Linux. Rendered items
// are asked for their icon id in one batch; each id's image is loaded once
// per size and shared by every file of that type. Until it arrives, and
// wherever the OS has none (remote places, the mock preview), the app's own
// drawing is shown.
import { iconIds, iconImage } from "./api";

/** Forget the path → id answers past this many (a few huge folders' worth). */
const MAX_PATHS = 20000;

/** path → icon id, or null when the OS has none. */
const ids = new Map<string, string | null>();
/** `${id}\n${size}` → object URL, or null when the OS couldn't draw it. Few
 *  entries: one per file type and size, so nothing is evicted. */
const images = new Map<string, string | null>();
/** id → the last image loaded for it at any size, shown while another size loads. */
const anySize = new Map<string, string>();
const askingIds = new Set<string>();
/** Images on their way, and the paths that show them. */
const waiting = new Map<string, Set<string>>();
let listener: (paths: string[]) => void = () => {};

/** Pixel size to ask for: the shown size at screen resolution, in steps. */
export function nativeIconSize(shownAt: number): number {
  const px = shownAt * devicePixelRatio;
  return [32, 64, 128, 256].find((step) => px <= step) ?? 512;
}

/** The loaded icon for `path` at `size` (or at another size while that one
 *  loads): a URL, or undefined (none yet, or none at all). */
export function nativeIconUrl(path: string, size: number): string | undefined {
  const id = ids.get(path);
  if (!id) return undefined;
  return images.get(`${id}\n${size}`) || anySize.get(id);
}

/** Load the icons of `paths` at `size`; `onReady` gets the paths whose icon just arrived. */
export function loadNativeIcons(paths: string[], size: number, onReady: (paths: string[]) => void): void {
  listener = onReady;
  const unknown = paths.filter((path) => !ids.has(path) && !askingIds.has(path));
  requestImages(paths.filter((path) => ids.has(path)), size);
  if (!unknown.length) return;
  for (const path of unknown) askingIds.add(path);
  iconIds(unknown)
    .catch(() => unknown.map(() => null))
    .then((answers) => {
      if (ids.size + unknown.length > MAX_PATHS) ids.clear();
      unknown.forEach((path, i) => {
        askingIds.delete(path);
        ids.set(path, answers[i] ?? null);
      });
      // Types seen before are drawn at once; the others when they load.
      const ready = unknown.filter((path) => nativeIconUrl(path, size));
      if (ready.length) listener(ready);
      requestImages(unknown, size);
    });
}

function requestImages(paths: string[], size: number): void {
  for (const path of paths) {
    const id = ids.get(path);
    if (!id) continue;
    const key = `${id}\n${size}`;
    if (images.has(key)) continue;
    const shownBy = waiting.get(key);
    if (shownBy) {
      shownBy.add(path);
      continue;
    }
    waiting.set(key, new Set([path]));
    iconImage(id, size)
      .catch(() => new ArrayBuffer(0))
      .then((buf) => {
        const url = buf.byteLength ? URL.createObjectURL(new Blob([buf], { type: imageType(buf) })) : null;
        images.set(key, url);
        if (url) anySize.set(id, url);
        const shown = [...(waiting.get(key) ?? [])];
        waiting.delete(key);
        if (images.get(key)) listener(shown);
      });
  }
}

/** PNG from macOS, SVG (or a PNG) from a Linux icon theme. */
function imageType(buf: ArrayBuffer): string {
  const png = [0x89, 0x50, 0x4e, 0x47];
  const head = new Uint8Array(buf, 0, Math.min(4, buf.byteLength));
  return png.every((byte, i) => head[i] === byte) ? "image/png" : "image/svg+xml";
}

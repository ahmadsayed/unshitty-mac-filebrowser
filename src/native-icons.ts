// Icons from the operating system (src-tauri/src/file_icons.rs): the ones
// Finder shows on macOS, the desktop's icon theme on Linux. Rendered items
// are asked for their icon id in one batch; each id's image is loaded once
// per size and shared by every file of that type. Until it arrives, and
// wherever the OS has none (remote places, the mock preview), the app's own
// drawing is shown. Keys are file paths, or `appIconKey(app)` for the apps
// Open With offers.
import { appIconIds, iconIds, iconImage } from "./api";

/** Forget the key → id answers past this many (a few huge folders' worth). */
const MAX_KEYS = 20000;
const APP = "app:";

/** key → icon id, or null when the OS has none. */
const ids = new Map<string, string | null>();
const idsLoading = new Map<string, Promise<void>>();
/** `${id}\n${size}` → object URL, or null when the OS couldn't draw it. Few
 *  entries: one per file type (or app) and size, so nothing is evicted. */
const images = new Map<string, string | null>();
const imagesLoading = new Map<string, Promise<string | null>>();
/** id → the last image loaded for it at any size, shown while another size loads. */
const anySize = new Map<string, string>();
/** `${key}\n${size}` already being waited for by `loadNativeIcons`. */
const announcing = new Set<string>();

/** The key an app from Open With's list is cached under. */
export function appIconKey(app: string): string {
  return `${APP}${app}`;
}

/** Pixel size to ask for: the shown size at screen resolution, in steps. */
export function nativeIconSize(shownAt: number): number {
  const px = shownAt * devicePixelRatio;
  return [32, 64, 128, 256].find((step) => px <= step) ?? 512;
}

/** The loaded icon for `key` at `size` (or at another size while that one
 *  loads): a URL, or undefined (none yet, or none at all). */
export function nativeIconUrl(key: string, size: number): string | undefined {
  const id = ids.get(key);
  if (!id) return undefined;
  return images.get(`${id}\n${size}`) || anySize.get(id);
}

/** The icon for one key at `size`, loaded if needed: a URL, or null when the OS has none. */
export async function nativeIcon(key: string, size: number): Promise<string | null> {
  if (!ids.has(key) && !idsLoading.has(key)) askIds([key]);
  await idsLoading.get(key);
  const id = ids.get(key);
  return id ? image(id, size) : null;
}

/** Load the icons of rendered items at `size`, asking for their ids in one
 *  batch; `onReady` gets the keys whose icon just arrived. */
export function loadNativeIcons(keys: string[], size: number, onReady: (keys: string[]) => void): void {
  askIds(keys.filter((key) => !ids.has(key) && !idsLoading.has(key)));
  for (const key of keys) {
    const id = ids.get(key);
    // Known to have none, or drawn already at this size.
    if (id === null || (id && images.has(`${id}\n${size}`))) continue;
    const waiting = `${key}\n${size}`;
    if (announcing.has(waiting)) continue;
    announcing.add(waiting);
    void nativeIcon(key, size).then((url) => {
      announcing.delete(waiting);
      if (url) onReady([key]);
    });
  }
}

/** One request per kind: file paths and apps are named differently. */
function askIds(keys: string[]): void {
  const apps = keys.filter((key) => key.startsWith(APP));
  const paths = keys.filter((key) => !key.startsWith(APP));
  ask(paths, iconIds);
  ask(apps, (list) => appIconIds(list.map((key) => key.slice(APP.length))));
}

function ask(keys: string[], fetch: (keys: string[]) => Promise<(string | null)[]>): void {
  if (!keys.length) return;
  const answered = fetch(keys)
    .catch(() => keys.map(() => null))
    .then((answers) => {
      if (ids.size + keys.length > MAX_KEYS) ids.clear();
      keys.forEach((key, i) => {
        ids.set(key, answers[i] ?? null);
        idsLoading.delete(key);
      });
    });
  for (const key of keys) idsLoading.set(key, answered);
}

function image(id: string, size: number): Promise<string | null> {
  const key = `${id}\n${size}`;
  if (images.has(key)) return Promise.resolve(images.get(key) ?? null);
  let loading = imagesLoading.get(key);
  if (!loading) {
    loading = iconImage(id, size)
      .catch(() => new ArrayBuffer(0))
      .then((buf) => {
        const url = buf.byteLength ? URL.createObjectURL(new Blob([buf], { type: imageType(buf) })) : null;
        images.set(key, url);
        imagesLoading.delete(key);
        if (url) anySize.set(id, url);
        return url;
      });
    imagesLoading.set(key, loading);
  }
  return loading;
}

/** PNG from macOS, SVG (or a PNG) from a Linux icon theme. */
function imageType(buf: ArrayBuffer): string {
  const png = [0x89, 0x50, 0x4e, 0x47];
  const head = new Uint8Array(buf, 0, Math.min(4, buf.byteLength));
  return png.every((byte, i) => head[i] === byte) ? "image/png" : "image/svg+xml";
}

// Loads file thumbnails from the backend and keeps them as blob URLs.
// Requests run a few at a time and only for items that are on screen; the
// URL cache is bounded and frees the least recently shown images.
import { thumbnail } from "./api";

/**
 * Memory the cached images may use. Thumbnails are PNGs: photos average
 * ~30 KB at 128 px, ~120 KB at 256 px and ~450 KB at 512 px (Retina, largest
 * zoom), so a count limit alone let the cache reach hundreds of megabytes.
 * 64 MB still holds several screens of the largest thumbnails.
 */
const MAX_BYTES = 64 * 1024 * 1024;
/** Also bound the number of entries, which includes "no thumbnail" answers. */
const MAX_CACHED = 2000;
const CONCURRENCY = 4;

/** key → object URL and its size, or null when the file has no thumbnail. */
const cache = new Map<string, { url: string; bytes: number } | null>();
let cachedBytes = 0;
const pending = new Map<string, { path: string; size: number; listeners: ((url: string | null) => void)[] }>();
const queue: string[] = [];
let running = 0;

export function thumbKey(path: string, modified: number, bytes: number, size: number): string {
  return `${path}\n${modified}\n${bytes}\n${size}`;
}

/** The cached result: a URL, null for "no thumbnail", undefined if not loaded yet. */
export function cachedThumb(key: string): string | null | undefined {
  const hit = cache.get(key);
  if (hit === undefined) return undefined;
  // Refresh its position so thumbnails on screen are evicted last.
  cache.delete(key);
  cache.set(key, hit);
  return hit && hit.url;
}

/** Size of the cache, for checking the limits. */
export function thumbCacheStats(): { entries: number; bytes: number } {
  return { entries: cache.size, bytes: cachedBytes };
}

/** Ask for a thumbnail; `onReady` runs once it's loaded (with null if there is none). */
export function requestThumb(key: string, path: string, size: number, onReady: (url: string | null) => void): void {
  const entry = pending.get(key);
  if (entry) {
    entry.listeners.push(onReady);
    return;
  }
  pending.set(key, { path, size, listeners: [onReady] });
  queue.push(key);
  pump();
}

/** Forget queued requests that are no longer on screen. */
export function keepOnly(keys: Set<string>): void {
  for (let i = queue.length - 1; i >= 0; i--) {
    if (!keys.has(queue[i])) {
      pending.delete(queue[i]);
      queue.splice(i, 1);
    }
  }
}

function pump(): void {
  while (running < CONCURRENCY && queue.length) {
    const key = queue.shift()!;
    const job = pending.get(key);
    if (!job) continue;
    running++;
    thumbnail(job.path, job.size)
      .then((buf) => (buf.byteLength ? { url: URL.createObjectURL(new Blob([buf], { type: "image/png" })), bytes: buf.byteLength } : null))
      .catch(() => null)
      .then((thumb) => {
        running--;
        pending.delete(key);
        store(key, thumb);
        job.listeners.forEach((fn) => fn(thumb && thumb.url));
        pump();
      });
  }
}

function store(key: string, thumb: { url: string; bytes: number } | null): void {
  const old = cache.get(key);
  if (old) {
    cachedBytes -= old.bytes;
    URL.revokeObjectURL(old.url);
  }
  cache.delete(key);
  cache.set(key, thumb);
  if (thumb) cachedBytes += thumb.bytes;
  // Evict from the least recently shown end, but never the image just added.
  for (const [oldKey, oldThumb] of cache) {
    if (cachedBytes <= MAX_BYTES && cache.size <= MAX_CACHED) break;
    if (oldKey === key) break;
    cache.delete(oldKey);
    if (oldThumb) {
      cachedBytes -= oldThumb.bytes;
      URL.revokeObjectURL(oldThumb.url);
    }
  }
}

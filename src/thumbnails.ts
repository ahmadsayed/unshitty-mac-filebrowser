// Loads file thumbnails from the backend and keeps them as blob URLs.
// Requests run a few at a time and only for items that are on screen; the
// URL cache is bounded and frees the oldest images.
import { thumbnail } from "./api";

const MAX_CACHED = 600;
const CONCURRENCY = 4;

/** key → object URL, or null when the file has no thumbnail. */
const cache = new Map<string, string | null>();
const pending = new Map<string, { path: string; size: number; listeners: ((url: string | null) => void)[] }>();
const queue: string[] = [];
let running = 0;

export function thumbKey(path: string, modified: number, bytes: number, size: number): string {
  return `${path}\n${modified}\n${bytes}\n${size}`;
}

/** The cached result: a URL, null for "no thumbnail", undefined if not loaded yet. */
export function cachedThumb(key: string): string | null | undefined {
  const hit = cache.get(key);
  if (hit !== undefined) {
    // Refresh its position so busy thumbnails survive eviction.
    cache.delete(key);
    cache.set(key, hit);
  }
  return hit;
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
      .then((buf) => (buf.byteLength ? URL.createObjectURL(new Blob([buf], { type: "image/png" })) : null))
      .catch(() => null)
      .then((url) => {
        running--;
        pending.delete(key);
        store(key, url);
        job.listeners.forEach((fn) => fn(url));
        pump();
      });
  }
}

function store(key: string, url: string | null): void {
  cache.set(key, url);
  while (cache.size > MAX_CACHED) {
    const [oldKey, oldUrl] = cache.entries().next().value!;
    cache.delete(oldKey);
    if (oldUrl) URL.revokeObjectURL(oldUrl);
  }
}

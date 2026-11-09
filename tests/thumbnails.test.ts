// The thumbnail queue on its own, with the backend call held open so a
// request can be repeated, or cancelled, while it is still loading.
import { afterEach, expect, test, vi } from "vitest";

const calls: { path: string; ticket: number; resolve: (buf: ArrayBuffer) => void }[] = [];
const cancelled: number[] = [];
vi.mock("../src/api", () => ({
  thumbnail: (path: string, _size: number, ticket: number) =>
    new Promise<ArrayBuffer>((resolve) => calls.push({ path, ticket, resolve })),
  cancelThumbnails: async (tickets: number[]) => {
    cancelled.push(...tickets);
  },
}));

const { cachedThumb, keepOnly, requestThumb, thumbKey, thumbType } = await import("../src/thumbnails");

const flush = () => new Promise((r) => setTimeout(r, 0));

afterEach(async () => {
  keepOnly(new Set());
  for (const call of calls.splice(0)) call.resolve(new ArrayBuffer(0));
  cancelled.length = 0;
  await flush();
});

test("a thumbnail asked for again while it loads is fetched and announced once", async () => {
  // Each render of the view repeats the request for every visible item.
  const key = thumbKey("/a.jpg", 1, 10, 128);
  const ready: string[] = [];
  for (const render of ["first", "second", "third"]) requestThumb(key, "/a.jpg", 128, () => ready.push(render));
  expect(calls.map((c) => c.path)).toEqual(["/a.jpg"]);

  calls[0].resolve(new ArrayBuffer(8));
  await flush();
  // Only the latest render's item is still on screen to patch.
  expect(ready).toEqual(["third"]);
});

test("different thumbnails keep their own listeners", async () => {
  const ready: string[] = [];
  requestThumb(thumbKey("/b.jpg", 1, 10, 128), "/b.jpg", 128, () => ready.push("b"));
  requestThumb(thumbKey("/c.jpg", 1, 10, 128), "/c.jpg", 128, () => ready.push("c"));
  for (const call of calls.splice(0)) call.resolve(new ArrayBuffer(8));
  await flush();
  expect(ready.sort()).toEqual(["b", "c"]);
});

test("a running request that left the screen is cancelled and frees its slot", async () => {
  // The old folder's four thumbnails are being made when the view moves on.
  const old = ["/old/1.jpg", "/old/2.jpg", "/old/3.jpg", "/old/4.jpg"];
  for (const path of old) requestThumb(thumbKey(path, 1, 10, 128), path, 128, () => {});
  const next = thumbKey("/new/1.jpg", 1, 10, 128);
  requestThumb(next, "/new/1.jpg", 128, () => {});
  expect(calls.map((c) => c.path)).toEqual(old);

  keepOnly(new Set([next]));
  expect(cancelled.sort()).toEqual(calls.slice(0, 4).map((c) => c.ticket).sort());
  expect(calls.map((c) => c.path)).toEqual([...old, "/new/1.jpg"]);
});

test("queued requests follow the order the view asks in, most wanted first", async () => {
  const busy = ["/busy/1.jpg", "/busy/2.jpg", "/busy/3.jpg", "/busy/4.jpg"].map((path) => {
    const key = thumbKey(path, 1, 10, 128);
    requestThumb(key, path, 128, () => {});
    return key;
  });
  // Asked for while scrolled up; now that row is overscan and "/b.jpg" is visible.
  const a = thumbKey("/a.jpg", 1, 10, 128);
  const b = thumbKey("/b.jpg", 1, 10, 128);
  requestThumb(a, "/a.jpg", 128, () => {});
  requestThumb(b, "/b.jpg", 128, () => {});
  keepOnly(new Set([...busy, b, a]));

  calls[0].resolve(new ArrayBuffer(8));
  await flush();
  expect(calls.at(-1)!.path).toBe("/b.jpg");
});

test("a cancelled request's answer is neither cached nor announced", async () => {
  const key = thumbKey("/gone.jpg", 1, 10, 128);
  const ready: (string | null)[] = [];
  requestThumb(key, "/gone.jpg", 128, (url) => ready.push(url));
  keepOnly(new Set());
  // The backend answers a cancelled request with no image.
  calls[0].resolve(new ArrayBuffer(0));
  await flush();
  expect(ready).toEqual([]);
  expect(cachedThumb(key)).toBeUndefined();

  // Back on screen, it is asked for again under a new ticket.
  requestThumb(key, "/gone.jpg", 128, (url) => ready.push(url));
  expect(calls.map((c) => c.path)).toEqual(["/gone.jpg", "/gone.jpg"]);
  expect(calls[1].ticket).not.toBe(calls[0].ticket);
});

test("a thumbnail is shown as the image type its bytes are", async () => {
  // The backend keeps decoded photos as JPEG, transparent pictures and the
  // desktop's own thumbnails as PNG.
  const types: string[] = [];
  vi.mocked(URL.createObjectURL).mockImplementation((blob) => {
    types.push((blob as Blob).type);
    return "blob:test";
  });
  const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xe0, 0, 0x10]).buffer;
  const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a]).buffer;
  requestThumb(thumbKey("/photo.jpg", 1, 10, 128), "/photo.jpg", 128, () => {});
  requestThumb(thumbKey("/logo.png", 1, 10, 128), "/logo.png", 128, () => {});
  calls[0].resolve(jpeg);
  calls[1].resolve(png);
  await flush();
  expect(types).toEqual(["image/jpeg", "image/png"]);
  expect(thumbType(new ArrayBuffer(2))).toBe("image/png");
});

// The thumbnail queue on its own, with the backend call held open so a
// request can be repeated while it is still loading.
import { afterEach, expect, test, vi } from "vitest";

const calls: { path: string; resolve: (buf: ArrayBuffer) => void }[] = [];
vi.mock("../src/api", () => ({
  thumbnail: (path: string) => new Promise<ArrayBuffer>((resolve) => calls.push({ path, resolve })),
}));

const { requestThumb, thumbKey } = await import("../src/thumbnails");

afterEach(async () => {
  for (const call of calls.splice(0)) call.resolve(new ArrayBuffer(0));
  await Promise.resolve();
});

const flush = () => new Promise((r) => setTimeout(r, 0));

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

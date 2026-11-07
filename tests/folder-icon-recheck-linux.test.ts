// On Linux a folder's icon from the icon theme doesn't depend on what it
// holds, so a folder's icon is asked for once: neither a visit again nor
// selecting it while its count is coming asks again.
import { beforeAll, expect, test, vi } from "vitest";
import { invoke, nativeIconMock } from "../dev/mock-tauri/core";
import { boot, click, goTo, hit, installFakeCanvas, item, key, settle } from "./app";

type MockCounts = { hold(hold: boolean): void; held(): number; reset(): void };
const counts = () => (globalThis as unknown as { __mockCounts: MockCounts }).__mockCounts;

const PROJECTS = "/Users/me/Documents/Projects";
const WEBSITE = `${PROJECTS}/website`;
let blobs = 0;
const nativeIcon = (name: string) => item(name).querySelector<HTMLImageElement>(".icon img.native-icon");
const askedFor = (path: string, from: number) => nativeIconMock.log.ids.slice(from).flat().filter((p) => p === path).length;

beforeAll(async () => {
  Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (X11; Linux x86_64)", configurable: true });
  installFakeCanvas();
  vi.mocked(URL.createObjectURL).mockImplementation(() => `blob:icon-${++blobs}`);
  nativeIconMock.enabled = true;
  await boot();
  await settle();
});

test("Linux asks for a folder's icon once: not on a visit again, not when selected", async () => {
  // The app took its Linux code paths.
  expect(document.documentElement.dataset.theme).toBe("gnome");
  const first = nativeIconMock.log.ids.length;
  await goTo(PROJECTS);
  await settle(20);
  expect(askedFor(WEBSITE, first)).toBe(1);
  const empty = nativeIcon("website")!.src;

  counts().hold(true);
  try {
    await goTo("/Users/me/Documents");
    const from = nativeIconMock.log.ids.length;
    await goTo(PROJECTS);
    await settle(20);
    expect(counts().held()).toBeGreaterThan(0);
    await invoke("create_file", { path: `${WEBSITE}/index.html` });
    await click(hit("website"));
    counts().hold(false);
    await settle(20);
    expect(askedFor(WEBSITE, from)).toBe(0);
    expect(nativeIcon("website")!.src).toBe(empty);
  } finally {
    counts().reset();
  }
  // Reloading (Ctrl+R) asks for nothing either.
  const before = nativeIconMock.log.ids.length;
  await key("r", { ctrl: true });
  await settle(20);
  expect(askedFor(WEBSITE, before)).toBe(0);
});

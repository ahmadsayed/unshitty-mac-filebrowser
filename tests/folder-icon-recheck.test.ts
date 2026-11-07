// On macOS a folder's OS icon shows whether it holds anything (Finder's full
// folder). Besides once per listing, a folder's icon is asked for again when
// its count is: selecting a folder whose count isn't known yet. A folder whose
// count is known isn't asked again when selected.
import { beforeAll, expect, test, vi } from "vitest";
import { invoke, nativeIconMock } from "../dev/mock-tauri/core";
import { boot, click, goTo, hit, installFakeCanvas, item, settle } from "./app";

type MockCounts = { hold(hold: boolean): void; held(): number; reset(): void };
const counts = () => (globalThis as unknown as { __mockCounts: MockCounts }).__mockCounts;

const PROJECTS = "/Users/me/Documents/Projects";
const WEBSITE = `${PROJECTS}/website`;
let blobs = 0;
const nativeIcon = (name: string) => item(name).querySelector<HTMLImageElement>(".icon img.native-icon");
/** How many icon id requests since `from` named `path`. */
const askedFor = (path: string, from: number) => nativeIconMock.log.ids.slice(from).flat().filter((p) => p === path).length;

beforeAll(async () => {
  installFakeCanvas();
  vi.mocked(URL.createObjectURL).mockImplementation(() => `blob:icon-${++blobs}`);
  nativeIconMock.enabled = true;
  await boot();
  await settle();
});

test("selecting a folder whose count is still coming asks for its icon again, so it follows what it holds", async () => {
  await goTo(PROJECTS);
  await settle(20);
  const empty = nativeIcon("website")!.src;
  const full = nativeIcon("unshitty-mac-filebrowser")!.src;
  expect(empty).not.toBe(full);

  counts().hold(true);
  try {
    await goTo("/Users/me/Documents");
    const from = nativeIconMock.log.ids.length;
    await goTo(PROJECTS);
    await settle(20);
    // Asked once while the folder was being shown.
    expect(askedFor(WEBSITE, from)).toBe(1);
    expect(counts().held()).toBeGreaterThan(0);
    // It fills up after the listing; selecting it asks for its count again.
    await invoke("create_file", { path: `${WEBSITE}/index.html` });
    await click(hit("website"));
    counts().hold(false);
    await settle(20);
    expect(askedFor(WEBSITE, from)).toBe(2);
    expect(nativeIcon("website")!.src).toBe(full);
  } finally {
    counts().reset();
  }
});

test("selecting a folder whose count is known asks for nothing", async () => {
  await goTo(PROJECTS);
  await settle(20);
  const from = nativeIconMock.log.ids.length;
  await click(hit("unshitty-mac-filebrowser"));
  await settle(20);
  expect(askedFor(`${PROJECTS}/unshitty-mac-filebrowser`, from)).toBe(0);
});

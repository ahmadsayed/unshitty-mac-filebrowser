// ItemIcons on its own (src/item-icons.ts), against the mock backend: which
// places have OS icons, the shown size, which picture an item shows, and
// which folders are asked for again because their count was.
import { beforeAll, expect, test, vi } from "vitest";
import { nativeIconMock } from "../dev/mock-tauri/core";
import type { Entry } from "../src/api";
import { GRID_SIZES, hasOsIcons, ItemIcons, LIST_SIZES, type IconView } from "../src/item-icons";
import { nativeIcon } from "../src/native-icons";

const NETWORK = "pseudo:network";
const entry = (path: string, is_dir = false): Entry =>
  ({ name: path.slice(path.lastIndexOf("/") + 1), path, is_dir, size: 0, modified: 0, is_hidden: false, is_symlink: false });

function setup(opts: { followsContents?: boolean; location?: string; view?: Partial<IconView> } = {}) {
  const here = { location: opts.location ?? "/Users/me/Documents" };
  const view: IconView = { view: "grid", gridZoom: 2, listZoom: 1, showThumbnails: false, ...opts.view };
  const redrawn: string[] = [];
  const icons = new ItemIcons({
    followsContents: opts.followsContents ?? true,
    location: () => here.location,
    view: () => view,
    folderKind: () => "plain",
    compare: (a, b) => a.name.localeCompare(b.name),
    redraw: (path) => redrawn.push(path),
  });
  return { icons, here, view, redrawn };
}

const settle = async (rounds = 6) => {
  for (let i = 0; i < rounds; i++) await new Promise((r) => setTimeout(r, 0));
};
const askedFor = (path: string, from: number) => nativeIconMock.log.ids.slice(from).flat().filter((p) => p === path).length;

beforeAll(() => {
  let blobs = 0;
  vi.mocked(URL.createObjectURL).mockImplementation(() => `blob:unit-${++blobs}`);
  nativeIconMock.enabled = true;
});

test.each<[string, string, string | undefined, boolean]>([
  ["a local file", "/Users/me/a.txt", "/Users/me", true],
  ["a local file, any place", "/Users/me/a.txt", undefined, true],
  ["an app key, any place", "app:/Applications/Calculator.app", undefined, true],
  ["anything shown in Network", "/Users/me/a.txt", NETWORK, false],
  ["an SSH server", "sftp://me@host/home", undefined, false],
  ["a received share", "share://abc/x", "/Users/me", false],
  ["Drive", "gdrive://me@example.com/x", undefined, false],
])("has OS icons: %s", (_label, path, loc, expected) => {
  expect(hasOsIcons(path, loc)).toBe(expected);
});

test("the shown size follows the view and its zoom", () => {
  const { icons, view } = setup();
  expect(icons.shownPx()).toBe(GRID_SIZES[2]);
  view.view = "list";
  expect(icons.shownPx()).toBe(LIST_SIZES[1]);
  view.listZoom = 0;
  expect(icons.shownPx()).toBe(LIST_SIZES[0]);
});

test("the picture: the drawing until the OS icon loads; a glyph for SSH servers in Network; never the OS icon in remote places", async () => {
  const { icons, here } = setup();
  const file = entry("/Users/me/Documents/Notes.md");
  expect(icons.base(file)).toEqual({ drawn: "plain" });
  icons.rendered([file], { start: 0, end: 0 });
  await settle(10);
  expect(icons.base(file)).toMatchObject({ native: expect.stringMatching(/^blob:unit-/) });

  here.location = NETWORK;
  expect(icons.base(entry("sftp://me@host/home", true))).toEqual({ glyph: "network-server" });
  expect(icons.base(file)).toEqual({ drawn: "plain" });
  here.location = "/Users/me/Documents";
  expect(icons.base(entry("share://abc/x.txt"))).toEqual({ drawn: "plain" });
});

test("a recounted folder's icon is asked for again where it follows its contents", async () => {
  const folder = "/Users/me/Documents/Projects";
  await nativeIcon(folder, 128);
  const mac = setup({ followsContents: true });
  const linux = setup({ followsContents: false });

  let from = nativeIconMock.log.ids.length;
  linux.icons.recounted([folder]);
  await settle();
  expect(askedFor(folder, from)).toBe(0);

  from = nativeIconMock.log.ids.length;
  mac.icons.recounted([folder, "sftp://me@host/home"]);
  await settle();
  expect(askedFor(folder, from)).toBe(1);
  expect(nativeIconMock.log.ids.slice(from).flat()).not.toContain("sftp://me@host/home");
});

test("a reload doesn't wait; navigating rechecks the first screenful's folders once", async () => {
  const folder = entry("/Users/me/Documents/Projects", true);
  await nativeIcon(folder.path, 128);
  const { icons } = setup();
  const from = nativeIconMock.log.ids.length;
  await icons.listingStarted("/Users/me/Documents", [folder]);
  expect(askedFor(folder.path, from)).toBe(1);
  // Already asked for this listing: rendering it doesn't ask again.
  icons.rendered([folder], { start: 0, end: 0 });
  await settle();
  expect(askedFor(folder.path, from)).toBe(1);
  // A reload forgets that, so the rendered folder is asked again.
  icons.listingReloaded();
  icons.rendered([folder], { start: 0, end: 0 });
  await settle();
  expect(askedFor(folder.path, from)).toBe(2);
  // Network isn't waited for, and asks nothing.
  await icons.listingStarted(NETWORK, [entry("sftp://me@host/home", true)]);
  expect(askedFor("sftp://me@host/home", from)).toBe(0);
});

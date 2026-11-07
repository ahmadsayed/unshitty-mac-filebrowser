// The markup of every item icon: the app's drawing, thumbnails, the SFTP
// server glyph and the emblems on top (link, versions, shared). The views'
// CSS targets these exact elements and classes, so they are pinned here as
// written, for each picture × emblem the app shows. The OS icons' markup is
// pinned in icon-markup-native.test.ts.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { entryIcon, type FolderKind } from "../src/icons";
import { $, $$, boot, click, closeOverlays, goTo, hit, installFakeCanvas, item, key, settle, type } from "./app";

const sftp = () => (globalThis as unknown as { __mockSftp: { reset(): void } }).__mockSftp;
const place = (label: string) => $$(".sidebar-row").find((row) => row.textContent?.trim() === label)!;
const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));
/** An item's icon markup, with image URLs (blob: counters) made constant. */
const markup = (name: string) => item(name).querySelector(".icon")!.innerHTML.replace(/ src="[^"]*"/g, ' src="URL"');
/** `s` as the DOM serializes it (`<use/>` comes back as `<use></use>`). */
const html = (s: string) => {
  const box = document.createElement("div");
  box.innerHTML = s;
  return box.innerHTML;
};

beforeAll(async () => {
  installFakeCanvas();
  await invoke("make_versioned_dir", { path: "/Users/me/Documents/Kept" });
  await invoke("make_versioned_dir", { path: "/Applications/Kept.app" });
  await invoke("share_folder", { path: "/Users/me/Documents/Projects" });
  await invoke("share_folder", { path: "/Users/me/Documents/Versioned" });
  await invoke("create_symlink", { path: "/Users/me/Pictures/Screenshots/Screenshot 2.png" });
  await boot();
  await settle();
});

// ---------------------------------------------------------------------------
// entryIcon, the app's drawing with the link and versions emblems
// ---------------------------------------------------------------------------

const entry = (name: string, is_dir: boolean, is_symlink = false) =>
  ({ name, path: `/x/${name}`, is_dir, size: 0, modified: 0, is_hidden: false, is_symlink });

test.each<[string, ReturnType<typeof entry>, FolderKind | undefined, boolean | undefined, string]>([
  ["a file", entry("a.txt", false), undefined, undefined,
    `<svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#fc-file-text"/></svg>`],
  ["a linked file", entry("a.png", false, true), "plain", false,
    `<svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#fc-file-image"/></svg><span class="emblem"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-emblem-symbolic-link"/></svg></span>`],
  ["a versioned file (badge)", entry("a.txt", false), "plain", true,
    `<svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#fc-file-text"/></svg><span class="emblem emblem-versions" title="Keeps versions"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-emblem-versions"/></svg></span>`],
  ["a folder of a kind", entry("Music", true), "music", false,
    `<svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#fc-folder-music"/></svg>`],
  ["a linked folder", entry("docs", true, true), "plain", false,
    `<svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#fc-folder-plain"/></svg><span class="emblem"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-emblem-symbolic-link"/></svg></span>`],
  ["a versioned folder (stacked)", entry("Kept", true), "documents", true,
    `<span class="versioned-icon" title="Keeps versions"><svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#fc-folder-back" transform="translate(5 -5)"/><use href="#fc-folder-documents"/></svg></span>`],
  ["a versioned linked folder (stacked, then the link)", entry("Kept", true, true), "plain", true,
    `<span class="versioned-icon" title="Keeps versions"><svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#fc-folder-back" transform="translate(5 -5)"/><use href="#fc-folder-plain"/></svg></span><span class="emblem"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-emblem-symbolic-link"/></svg></span>`],
  ["an app", entry("Calc.app", true), "plain", false,
    `<svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#fc-file-executable"/></svg>`],
  ["a versioned linked app (link, then the badge)", entry("Calc.app", true, true), "plain", true,
    `<svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#fc-file-executable"/></svg><span class="emblem"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-emblem-symbolic-link"/></svg></span><span class="emblem emblem-versions" title="Keeps versions"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-emblem-versions"/></svg></span>`],
])("entryIcon: %s", (_, e, kind, versioned, expected) => {
  expect(entryIcon(e, kind, versioned)).toBe(expected);
});

// ---------------------------------------------------------------------------
// Items in the view, with the app's drawings (no OS icons in this file)
// ---------------------------------------------------------------------------

const LINK = `<span class="emblem"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-emblem-symbolic-link"/></svg></span>`;
const BADGE = `<span class="emblem emblem-versions" title="Keeps versions"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-emblem-versions"/></svg></span>`;
const SHARED = `<span class="emblem"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-network-workgroup"/></svg></span>`;
const drawn = (id: string) => `<svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#${id}"/></svg>`;
const stacked = (id: string) =>
  `<span class="versioned-icon" title="Keeps versions"><svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#fc-folder-back" transform="translate(5 -5)"/><use href="#${id}"/></svg></span>`;

test("home: a file, a folder of a kind, a linked folder", async () => {
  await goTo("/Users/me");
  await settle();
  expect(markup("main.rs")).toBe(html(drawn("fc-file-code")));
  expect(markup("Documents")).toBe(html(drawn("fc-folder-documents")));
  expect(markup("link-to-docs")).toBe(html(drawn("fc-folder-plain") + LINK));
});

test("Documents: versioned, shared, and versioned and shared folders", async () => {
  await goTo("/Users/me/Documents");
  await settle();
  expect(markup("Notes.md")).toBe(html(drawn("fc-file-text")));
  expect(markup("Kept")).toBe(html(stacked("fc-folder-plain")));
  expect(markup("Projects")).toBe(html(drawn("fc-folder-plain") + SHARED));
  expect(markup("Versioned")).toBe(html(stacked("fc-folder-plain") + SHARED));
});

test("the list view draws the same icons", async () => {
  await key("2", { mod: true });
  await settle();
  expect(markup("Versioned")).toBe(html(stacked("fc-folder-plain") + SHARED));
  expect(markup("Kept")).toBe(html(stacked("fc-folder-plain")));
  await key("1", { mod: true });
  await settle();
});

test("apps: an app, and a versioned app shows the badge, not the stack", async () => {
  await goTo("/Applications");
  await settle();
  expect(markup("Calculator.app")).toBe(html(drawn("fc-file-executable")));
  expect(markup("Kept.app")).toBe(html(drawn("fc-file-executable") + BADGE));
});

test("thumbnails: a picture, and a linked picture keeps the link emblem", async () => {
  await goTo("/Users/me/Pictures/Screenshots");
  // The mock takes 150 ms to draw a picture.
  await pause(400);
  await settle();
  expect(markup("Screenshot 1.png")).toBe(html(`<img class="thumb" src="URL" alt="" draggable="false">`));
  expect(markup("Link to Screenshot 2.png")).toBe(html(`<img class="thumb" src="URL" alt="" draggable="false">${LINK}`));
});

test("dialogs draw the item with its emblems", async () => {
  await goTo("/Users/me");
  await settle();
  await click(hit("link-to-docs"));
  await key("i", { mod: true });
  await settle(20);
  expect($(".properties-icon").innerHTML).toBe(html(drawn("fc-folder-plain") + LINK));
  await closeOverlays();
});

test("Network: an SSH server shows the server glyph alone, a received share the shared emblem", async () => {
  sftp().reset();
  await invoke("add_received", { link: `share://${"mockreceived".padEnd(64, "a")}` });
  await click(place("Network"));
  await settle(20);
  await type($<HTMLInputElement>(".network-link"), "user@host");
  await click($(".network-paste .btn"));
  await settle(30);
  await click(place("Network"));
  await settle(20);
  expect(markup("user@host")).toBe(html(
    `<span class="server-glyph"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-network-server"/></svg></span>`
  ));
  expect(markup("Photos")).toBe(html(drawn("fc-folder-plain") + SHARED));
});

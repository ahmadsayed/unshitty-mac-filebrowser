// The markup of items and dialogs drawn with the OS's icons, and the app's
// emblems on top (link, versions, shared): pinned as written, since the
// views' CSS targets it. The app's own drawings are in icon-markup.test.ts.
import { beforeAll, expect, test, vi } from "vitest";
import { invoke, nativeIconMock } from "../dev/mock-tauri/core";
import { $, $$, boot, chooseMenu, click, closeOverlays, goTo, hit, installFakeCanvas, item, key, mouse, settle } from "./app";

/** Markup with image URLs (blob: counters) made constant. */
const constant = (s: string) => s.replace(/ src="[^"]*"/g, ' src="URL"');
const markup = (name: string) => constant(item(name).querySelector(".icon")!.innerHTML);
/** `s` as the DOM serializes it (`<use/>` comes back as `<use></use>`). */
const html = (s: string) => {
  const box = document.createElement("div");
  box.innerHTML = s;
  return box.innerHTML;
};

const OS = `<img class="native-icon" src="URL" alt="" draggable="false">`;
const LINK = `<span class="emblem"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-emblem-symbolic-link"/></svg></span>`;
const BADGE = `<span class="emblem emblem-versions" title="Keeps versions"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-emblem-versions"/></svg></span>`;
const SHARED = `<span class="emblem"><svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-network-workgroup"/></svg></span>`;

let blobs = 0;

beforeAll(async () => {
  installFakeCanvas();
  vi.mocked(URL.createObjectURL).mockImplementation(() => `blob:icon-${++blobs}`);
  nativeIconMock.enabled = true;
  await invoke("make_versioned_dir", { path: "/Applications/Kept.app" });
  await invoke("share_folder", { path: "/Users/me/Documents/Projects" });
  await invoke("share_folder", { path: "/Users/me/Documents/Versioned" });
  await boot();
  await settle();
});

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

test("home: a file, and a linked folder keeps the link emblem", async () => {
  await goTo("/Users/me");
  await settle(20);
  expect(markup("main.rs")).toBe(OS);
  expect(markup("link-to-docs")).toBe(html(OS + LINK));
});

test("Documents: a versioned folder has the badge, not the stack; shared ones the shared emblem last", async () => {
  await goTo("/Users/me/Documents");
  await settle(20);
  expect(markup("Notes.md")).toBe(OS);
  expect(markup("Projects")).toBe(html(OS + SHARED));
  expect(markup("Versioned")).toBe(html(OS + BADGE + SHARED));
});

test("apps: an app's own icon, and the badge on a versioned one", async () => {
  await goTo("/Applications");
  await settle(20);
  expect(markup("Calculator.app")).toBe(OS);
  expect(markup("Kept.app")).toBe(html(OS + BADGE));
});

test("Properties swaps in the OS icon and keeps the link emblem", async () => {
  await goTo("/Users/me");
  await settle(20);
  await click(hit("link-to-docs"));
  await key("i", { mod: true });
  await settle(20);
  expect(constant($(".properties-icon").innerHTML)).toBe(html(OS + LINK));
  await closeOverlays();
});

test("the preview of a folder swaps in the OS icon", async () => {
  await goTo("/Users/me/Documents");
  await settle(20);
  await click(hit("Projects"));
  await key(" ");
  await settle(20);
  expect(constant($("#preview-body .preview-empty-icon").innerHTML)).toBe(OS);
  await key("Escape");
  await settle();
});

test("Open With swaps in each app's icon", async () => {
  await goTo("/Users/me/Documents");
  await settle(20);
  await rightClick(hit("Notes.md"));
  await chooseMenu("Open With…");
  await settle(20);
  const icons = $$(".app-choice .app-choice-icon");
  expect(icons.length).toBeGreaterThan(1);
  for (const icon of icons) expect(constant(icon.innerHTML)).toBe(OS);
  await closeOverlays();
});

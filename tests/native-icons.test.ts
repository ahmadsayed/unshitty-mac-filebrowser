// Icons from the OS (Finder's on macOS, the icon theme on Linux) replace
// the app's own drawings once loaded. Rendered items ask for their icon ids
// in one batch, and each id's image is loaded once per size, however many
// files share it. Badges stay the app's; thumbnails still win; remote places
// keep the app's icons.
import { beforeAll, expect, test, vi } from "vitest";
import { nativeIconMock } from "../dev/mock-tauri/core";
import { $, $$, boot, chooseMenu, click, closeOverlays, goTo, hit, installFakeCanvas, item, key, mouse, settle } from "./app";

let blobs = 0;
const place = (label: string) => $$(".sidebar-row").find((row) => row.textContent?.trim() === label)!;
const nativeIcon = (name: string) => item(name).querySelector<HTMLImageElement>(".icon img.native-icon");
const imagesFor = (id: string) => nativeIconMock.log.images.filter((request) => request.startsWith(`${id}@`));

beforeAll(async () => {
  installFakeCanvas();
  vi.mocked(URL.createObjectURL).mockImplementation(() => `blob:icon-${++blobs}`);
  nativeIconMock.enabled = true;
  await boot();
  await settle();
});

test("rendered items show the OS icon, loaded once per type", async () => {
  await goTo("/Users/me/Documents");
  // Ids first, then the images.
  await settle(20);
  for (const name of ["Projects", "Versioned", "Quarterly report.pdf", "Budget 2026.xlsx", "Notes.md"]) {
    expect(nativeIcon(name), name).not.toBeNull();
  }
  // Both folders show the one folder image, loaded once for Home's folders.
  expect(nativeIcon("Projects")!.src).toBe(nativeIcon("Versioned")!.src);
  expect(imagesFor("type:public.folder")).toHaveLength(1);
  expect(nativeIcon("Quarterly report.pdf")!.src).not.toBe(nativeIcon("Projects")!.src);
  // The app's drawing is gone for them.
  expect(item("Projects").querySelector("svg.fc")).toBeNull();
});

test("the app's badges stay on OS icons", async () => {
  await goTo("/Users/me/Documents");
  await settle();
  expect(item("Versioned").querySelector(".emblem-versions")).not.toBeNull();
  await goTo("/Users/me");
  await settle();
  expect(nativeIcon("link-to-docs")).not.toBeNull();
  expect(item("link-to-docs").querySelector(".emblem")).not.toBeNull();
});

test("a folder visited again asks for nothing new", async () => {
  await goTo("/Users/me/Documents");
  await settle();
  const asked = nativeIconMock.log.ids.length;
  const drawn = nativeIconMock.log.images.length;
  await goTo("/Users/me");
  await settle();
  await goTo("/Users/me/Documents");
  await settle();
  // Home asks for its own items; Documents is answered from the cache.
  expect(nativeIconMock.log.ids.slice(asked).flat().some((p) => p.startsWith("/Users/me/Documents/"))).toBe(false);
  expect(nativeIconMock.log.images.slice(drawn).some((r) => r.startsWith("type:public.folder@"))).toBe(false);
});

test("apps get their own icon", async () => {
  await goTo("/Applications");
  await settle();
  expect(nativeIcon("Calculator.app")).not.toBeNull();
  expect(imagesFor("path:/Applications/Calculator.app")).toHaveLength(1);
});

test("a zoom asks for the new size and keeps showing the old one meanwhile", async () => {
  await goTo("/Users/me/Documents");
  await settle();
  const before = imagesFor("type:public.folder").length;
  await key("+", { mod: true });
  // Drawn at once from the size already loaded, not the app's drawing.
  expect(nativeIcon("Projects")).not.toBeNull();
  await settle();
  expect(imagesFor("type:public.folder").length).toBe(before + 1);
  await key("0", { mod: true });
  await settle();
});

test("thumbnails still win over the OS icon", async () => {
  await goTo("/Users/me/Pictures/Screenshots");
  // The mock's thumbnails take 150 ms.
  await new Promise((r) => setTimeout(r, 400));
  await settle();
  const first = $$("#view .item .icon")[0];
  expect(first.querySelector("img.thumb")).not.toBeNull();
  expect(first.querySelector("img.native-icon")).toBeNull();
});

test("Network keeps the app's icons and doesn't ask", async () => {
  const asked = nativeIconMock.log.ids.length;
  await click(place("Network"));
  await settle(20);
  expect(nativeIconMock.log.ids.slice(asked).flat()).toEqual([]);
  expect($$("#view .item img.native-icon")).toHaveLength(0);
});

// Dialogs ask for their one icon and swap it in when it arrives.

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

test("Open With shows each app's own icon", async () => {
  await goTo("/Users/me/Documents");
  await settle();
  await rightClick(hit("Notes.md"));
  await chooseMenu("Open With…");
  await settle(20);
  const choices = $$(".app-choice");
  expect(choices.length).toBeGreaterThan(1);
  for (const choice of choices) expect(choice.querySelector(".app-choice-icon img.native-icon"), choice.textContent!).not.toBeNull();
  expect(nativeIconMock.log.ids.at(-1)!.every((key) => key.startsWith("app:/Applications/"))).toBe(true);
  // Apps aren't files: each is drawn per app.
  const srcs = choices.map((choice) => choice.querySelector("img")!.src);
  expect(new Set(srcs).size).toBe(srcs.length);
  await closeOverlays();
});

test("Properties shows the OS icon and keeps the link badge", async () => {
  await goTo("/Users/me");
  await settle();
  await click(hit("link-to-docs"));
  await key("i", { mod: true });
  await settle(20);
  const icon = $(".properties-icon");
  expect(icon.querySelector("img.native-icon")).not.toBeNull();
  expect(icon.querySelector("svg.fc")).toBeNull();
  expect(icon.querySelector(".emblem")).not.toBeNull();
  await closeOverlays();
});

test("the preview of a folder shows its OS icon", async () => {
  await goTo("/Users/me/Documents");
  await settle();
  await click(hit("Projects"));
  await key(" ");
  await settle(20);
  expect($("#preview-body .preview-empty img.native-icon")).not.toBeNull();
  await key("Escape");
  await settle();
});

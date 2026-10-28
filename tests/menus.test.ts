// Context menus for items and the background in normal folders, Recent,
// Starred and the Trash; the main, view and location menus; keyboard
// navigation inside a menu; Restore From Trash and Properties.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, chooseMenu, click, closeOverlays, dialogHeading, goTo, item, key, menuLabels, mouse, names, respond, selectedNames, settle, title, hit } from "./app";

const HOME = "/Users/me";

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

const place = (label: string) => $$(".sidebar-row").find((r) => r.textContent?.trim() === label)!;

test("a file's menu in a folder", async () => {
  await goTo(HOME);
  await rightClick(item("main.rs"));
  expect(menuLabels()).toEqual(["Open", "Preview", "Open With…", "Cut", "Copy", "Duplicate", "Create Link", "Compress…", "Rename…", "Star", "Move to Trash", "Properties"]);
  await closeOverlays();
});

test("a folder's menu adds bookmarking and Paste Into Folder", async () => {
  await goTo(HOME);
  await rightClick(item("Empty folder"));
  expect(menuLabels()).toEqual(expect.arrayContaining(["Add to Bookmarks", "Paste Into Folder"]));
  expect($$(".popover .menu-item").find((b) => b.textContent === "Paste Into Folder")?.hasAttribute("disabled")).toBe(true);
  await chooseMenu("Add to Bookmarks");
  expect($("#sidebar-bookmarks").textContent).toContain("Empty folder");
  await rightClick(item("Empty folder"));
  await chooseMenu("Remove from Bookmarks");
  expect($("#sidebar-bookmarks").textContent).not.toContain("Empty folder");
});

test("an app bundle offers Show Package Contents", async () => {
  await goTo("/Applications");
  await rightClick(item("Calculator.app"));
  await chooseMenu("Show Package Contents");
  expect(title()).toBe("Calculator.app");
});

test("several items: Rename and Properties are disabled", async () => {
  await goTo(`${HOME}/Documents`);
  await key("a", { mod: true });
  await rightClick(item(names()[0]));
  const disabled = $$(".popover .menu-item:disabled").map((b) => b.textContent);
  expect(disabled).toEqual(["Preview", "Open With…", "Create Link", "Properties"]);
  await chooseMenu("Star");
  await rightClick(item(names()[0]));
  expect(menuLabels()).toContain("Unstar");
  await chooseMenu("Unstar");
});

test("Recent: Open Item Location and Remove from Recent", async () => {
  await click(place("Recent"));
  await settle(10);
  expect(title()).toBe("Recent");
  const first = names()[0];
  await rightClick(item(first));
  expect(menuLabels()).toEqual(["Open", "Preview", "Open With…", "Open Item Location", "Cut", "Copy", expect.stringMatching(/Star|Unstar/), "Move to Trash", "Remove from Recent", "Properties"]);
  await chooseMenu("Remove from Recent");
  expect(names()).not.toContain(first);
  await rightClick(item(names()[0]));
  await chooseMenu("Open Item Location");
  expect(title()).not.toBe("Recent");
  expect(selectedNames().length).toBe(1);
});

test("Starred: its own menu, and Open Item Location", async () => {
  await click(place("Starred"));
  await settle(10);
  expect(title()).toBe("Starred");
  await rightClick(item(names()[0]));
  expect(menuLabels()).toEqual(["Open", "Preview", "Open With…", "Open Item Location", "Cut", "Copy", "Unstar", "Move to Trash", "Properties"]);
  await closeOverlays();
});

test("Trash: Restore From Trash and Delete Permanently", async () => {
  await invoke("make_dir", { path: `${HOME}/Restorable` });
  await invoke("trash_entries", { paths: [`${HOME}/Restorable`] });
  await goTo(`${HOME}/.Trash`);
  await rightClick(item("unknown origin.zip"));
  const restore = $$(".popover .menu-item").find((b) => b.textContent === "Restore From Trash")!;
  expect(restore.hasAttribute("disabled")).toBe(true);
  await closeOverlays();
  await rightClick(item("Restorable"));
  await chooseMenu("Restore From Trash");
  expect(names()).not.toContain("Restorable");
  expect(await invoke("path_exists", { path: `${HOME}/Restorable` })).toBe(true);
  await rightClick(item("unknown origin.zip"));
  await chooseMenu("Delete Permanently…");
  expect(dialogHeading()).toMatch(/permanently/i);
  await respond("Delete");
  expect(names()).not.toContain("unknown origin.zip");
});

test("restoring onto an existing item reports it", async () => {
  await invoke("make_dir", { path: `${HOME}/Twice` });
  await invoke("trash_entries", { paths: [`${HOME}/Twice`] });
  await invoke("make_dir", { path: `${HOME}/Twice` });
  await goTo(`${HOME}/.Trash`);
  await key("F5"); // already showing the Trash: reload to see the new item
  await rightClick(item("Twice"));
  await chooseMenu("Restore From Trash");
  expect($$(".toast").map((t) => t.textContent).join(" ")).toMatch(/Could not restore/);
});

test("the background menu", async () => {
  await goTo(HOME);
  await rightClick($("#view"));
  expect(menuLabels()).toEqual(["New Folder…", "New Document…", "Share This Folder…", "Paste", "Select All"]);
  await chooseMenu("Select All");
  expect(selectedNames().length).toBe(names().length);
  await key("Escape");
});

test("the Trash's background menu empties it", async () => {
  await goTo(`${HOME}/.Trash`);
  await rightClick($("#view"));
  await chooseMenu("Empty Trash…");
  await respond("Empty Trash");
  expect(names()).toEqual([]);
});

test("menus move focus with the arrow keys and skip disabled items", async () => {
  await goTo(HOME);
  await rightClick(item("Empty folder"));
  const menu = $(".popover .menu");
  const enabled = $$(".popover .menu-item:not(:disabled)");
  await key("ArrowDown", { target: menu });
  expect(document.activeElement).toBe(enabled[0]);
  await key("ArrowDown", { target: menu });
  expect(document.activeElement).toBe(enabled[1]);
  await key("ArrowUp", { target: menu });
  await key("ArrowUp", { target: menu });
  expect(document.activeElement).toBe(enabled[enabled.length - 1]);
  await key("x", { target: menu }); // other keys are ignored
  await closeOverlays();
});

test("the main menu and the view menu show check and radio items", async () => {
  await goTo(HOME);
  await click($("#btn-main-menu"));
  expect(menuLabels().length).toBeGreaterThan(2);
  await closeOverlays();
  await click($("#btn-view-menu"));
  expect($$(".popover [role=menuitemradio]").length).toBeGreaterThan(0);
  await chooseMenu("Size");
  const sizes = names();
  await click($("#btn-view-menu"));
  await chooseMenu("Z–A");
  expect(names()).not.toEqual(sizes);
  await click($("#btn-view-menu"));
  await chooseMenu("A–Z");
});

test("the main menu's About item names the app File Browser", async () => {
  await goTo(HOME);
  await click($("#btn-main-menu"));
  expect(menuLabels()).toContain("About File Browser");
  await chooseMenu("About File Browser");
  expect(dialogHeading()).toBe("File Browser");
  expect($(".dialog-layer").textContent).toContain("for macOS and Linux");
  await closeOverlays();
});

test("Properties of a folder shows its size", async () => {
  await goTo(HOME);
  await rightClick(item("Desktop"));
  await chooseMenu("Properties");
  await settle(20);
  expect(dialogHeading()).toBe("Properties");
  expect($(".dialog-layer").textContent).toMatch(/bytes|kB/);
  await closeOverlays();
});

const popover = () => document.querySelector<HTMLElement>(".popover");
const fades = () => !popover()!.classList.contains("no-animation");
const layer = () => $(".popover-layer");

test("a menu opened from nothing fades in, however soon after the last one closed", async () => {
  await goTo(HOME);
  await rightClick(item("main.rs"));
  expect(fades()).toBe(true);
  await key("Escape", { target: layer() });
  await rightClick(item("Empty folder"));
  expect(fades()).toBe(true);
  // A press that only dismisses the menu doesn't make the next one instant.
  await mouse(layer(), "mousedown", { clientX: 50, clientY: 50 });
  await mouse(document.body, "mouseup");
  await rightClick(item("main.rs"));
  expect(fades()).toBe(true);
  await closeOverlays();
});

test("a right-click over an open menu replaces it without the fade", async () => {
  await goTo(HOME);
  await rightClick(item("main.rs"));
  await mouse(layer(), "mousedown", { button: 2, clientX: 50, clientY: 50 });
  await mouse(hit("Empty folder"), "contextmenu", { button: 2 });
  expect(menuLabels()).toContain("Paste Into Folder");
  expect(fades()).toBe(false);
  await closeOverlays();
  // Header menus too.
  await click($("#btn-main-menu"));
  await mouse(layer(), "mousedown", { button: 2, clientX: 50, clientY: 50 });
  await mouse(hit("main.rs"), "contextmenu", { button: 2 });
  expect(menuLabels()).toContain("Rename…");
  expect(fades()).toBe(false);
  await closeOverlays();
});

test("clicking a menu button while its menu is open closes it", async () => {
  await goTo(HOME);
  const button = $("#btn-main-menu");
  await click(button);
  expect(popover()).not.toBeNull();
  // The backdrop takes the press; the click that follows lands on the button.
  await mouse(layer(), "mousedown");
  await mouse(button, "mouseup");
  await mouse(button, "click");
  expect(popover()).toBeNull();
  expect(button.classList.contains("open")).toBe(false);
  await click(button);
  expect(popover()).not.toBeNull();
  await closeOverlays();
});

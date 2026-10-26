// Drag and drop inside the app and from other apps: drop targets (items,
// sidebar, breadcrumbs, the view), move vs copy, Trash, refusals, Escape,
// spring-loaded folders and auto-scroll.
import { afterEach, beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, goTo, item, key, mouse, names, settle, title, hit } from "./app";

const HOME = "/Users/me";
let pointAt: Element | null = null;

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
  document.elementFromPoint = () => pointAt;
  await invoke("make_dir", { path: `${HOME}/D` });
  await invoke("make_dir", { path: `${HOME}/D/Inbox` });
  for (const name of ["one.txt", "two.txt", "three.txt", "four.txt"]) {
    await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/D/${name}`, overwrite: false });
  }
});

afterEach(() => {
  pointAt = null;
});

const sidebarRow = (location: string) => $$(".sidebar-row").find((r) => r.dataset.location === location)!;

/** Press on `name`, move far enough to start a drag, hover `target`, release. */
async function dragTo(name: string, target: Element | null, mods: MouseEventInit = {}, release = true): Promise<void> {
  await mouse(hit(name), "mousedown", { clientX: 50, clientY: 60 });
  pointAt = target;
  window.dispatchEvent(new MouseEvent("mousemove", { clientX: 300, clientY: 300, ...mods }));
  await settle(10);
  window.dispatchEvent(new MouseEvent("mousemove", { clientX: 301, clientY: 301, ...mods }));
  await settle(10);
  if (release) {
    window.dispatchEvent(new MouseEvent("mouseup", { clientX: 301, clientY: 301, ...mods }));
    await settle(20);
  }
}

test("dropping a file on a folder moves it there", async () => {
  await goTo(`${HOME}/D`);
  await dragTo("one.txt", item("Inbox"));
  expect(names()).not.toContain("one.txt");
  expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/one.txt` })).toBe(true);
});

test("⌥ while dropping copies instead, and the ghost shows +", async () => {
  await goTo(`${HOME}/D`);
  await dragTo("two.txt", item("Inbox"), { altKey: true }, false);
  expect($(".drag-ghost .drag-op").hidden).toBe(false);
  expect(item("Inbox").classList.contains("drop-target")).toBe(true);
  window.dispatchEvent(new MouseEvent("mouseup", { altKey: true }));
  await settle(20);
  expect(names()).toContain("two.txt");
  expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/two.txt` })).toBe(true);
});

test("pressing a modifier key during a drag updates the operation", async () => {
  await goTo(`${HOME}/D`);
  await dragTo("three.txt", item("Inbox"), {}, false);
  expect($(".drag-ghost .drag-op").hidden).toBe(true);
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Alt", altKey: true }));
  expect($(".drag-ghost .drag-op").hidden).toBe(false);
  window.dispatchEvent(new KeyboardEvent("keyup", { key: "Alt" }));
  expect($(".drag-ghost .drag-op").hidden).toBe(true);
  // Escape cancels: nothing moves.
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
  await settle(10);
  expect(document.querySelector(".drag-ghost")).toBeNull();
  window.dispatchEvent(new MouseEvent("mouseup"));
  await settle(10);
  expect(names()).toContain("three.txt");
});

test("dropping on the Trash in the sidebar trashes", async () => {
  await goTo(`${HOME}/D`);
  await dragTo("three.txt", sidebarRow(`${HOME}/.Trash`));
  expect(names()).not.toContain("three.txt");
});

test("drops that would do nothing are refused", async () => {
  await goTo(`${HOME}/D`);
  // On the view of its own folder: moving there does nothing.
  await dragTo("four.txt", $(".view-overlay"), {}, false);
  expect(document.documentElement.style.cursor).toBe("no-drop");
  window.dispatchEvent(new MouseEvent("mouseup"));
  await settle(10);
  // A folder onto itself.
  await dragTo("Inbox", item("Inbox"), {}, false);
  expect(document.documentElement.style.cursor).toBe("no-drop");
  window.dispatchEvent(new MouseEvent("mouseup"));
  await settle(10);
  // Recent in the sidebar isn't a folder.
  await dragTo("four.txt", $$(".sidebar-row").find((r) => r.textContent?.includes("Recent")) ?? null);
  // Nothing under the pointer.
  await dragTo("four.txt", null);
  expect(names()).toEqual(expect.arrayContaining(["four.txt", "Inbox"]));
});

test("dropping on a breadcrumb moves up the tree; ⌘ forces a move", async () => {
  await goTo(`${HOME}/D/Inbox`);
  const crumb = $$(".crumb").find((c) => c.dataset.path === `${HOME}/D`)!;
  await dragTo("one.txt", crumb, { metaKey: true });
  expect(names()).not.toContain("one.txt");
  expect(await invoke("path_exists", { path: `${HOME}/D/one.txt` })).toBe(true);
});

test("hovering a folder for a second opens it (spring-loaded)", async () => {
  await goTo(`${HOME}/D`);
  await dragTo("four.txt", item("Inbox"), {}, false);
  await new Promise((r) => setTimeout(r, 1100));
  await settle(10);
  expect(title()).toBe("Inbox");
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
  await settle(10);
});

test("dragging near the bottom edge scrolls the view", async () => {
  await goTo(`${HOME}/D`);
  const scroller = $("#view-scroll");
  scroller.scrollTop = 0;
  await mouse(hit("four.txt"), "mousedown", { clientX: 50, clientY: 60 });
  // jsdom's rects are all zero, so y = 10 sits inside the bottom edge band.
  window.dispatchEvent(new MouseEvent("mousemove", { clientX: 0, clientY: 10 }));
  await new Promise((r) => setTimeout(r, 120));
  expect(scroller.scrollTop).toBeGreaterThan(0);
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
  await settle(10);
});

test("files dropped from another app are moved on the same disk and copied from another", async () => {
  await goTo(`${HOME}/D`);
  const drop = (window as unknown as { __mockDrop: (paths: string[], x: number, y: number) => void }).__mockDrop;
  pointAt = item("Inbox");
  drop([`${HOME}/Desktop/todo.txt`], 10, 10);
  await settle(30);
  expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/todo.txt` })).toBe(true);
  expect(await invoke("path_exists", { path: `${HOME}/Desktop/todo.txt` })).toBe(false);
  await invoke("make_dir", { path: "/Volumes/Backup Drive/photo" });
  pointAt = item("Inbox"); // the view re-rendered after the first drop
  drop(["/Volumes/Backup Drive/photo"], 10, 10);
  await settle(30);
  expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/photo` })).toBe(true);
  expect(await invoke("path_exists", { path: "/Volumes/Backup Drive/photo" })).toBe(true);
  // Dropped on nothing: ignored.
  pointAt = null;
  drop([`${HOME}/main.rs`], 10, 10);
  await settle(20);
  expect(await invoke("path_exists", { path: `${HOME}/main.rs` })).toBe(true);
});

test("the Google Drive row is not a drop target while disconnected", async () => {
  await goTo(`${HOME}/D`);
  const driveRow = $$(".sidebar-row").find((r) => r.textContent?.includes("Google Drive"))!;
  await dragTo("four.txt", driveRow);
  expect(names()).toContain("four.txt");
  expect(await invoke("path_exists", { path: "/four.txt" })).toBe(false);
});

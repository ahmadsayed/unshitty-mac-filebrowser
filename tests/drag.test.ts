// Drag and drop inside the app and from other apps: drop targets (items,
// sidebar, breadcrumbs, the view), move vs copy, Trash, refusals, Escape,
// spring-loaded folders and auto-scroll.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { DRAG_IMAGE } from "../src/drag-out";
import { $, $$, boot, goTo, item, key, mouse, names, respond, settle, title, hit, toasts } from "./app";

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

const mockDrop = (paths: string[], x: number, y: number) =>
  (window as unknown as { __mockDrop: (paths: string[], x: number, y: number) => void }).__mockDrop(paths, x, y);

test("files dropped from another app are copied, on the same disk too", async () => {
  await goTo(`${HOME}/D`);
  const drop = mockDrop;
  pointAt = item("Inbox");
  drop([`${HOME}/Desktop/todo.txt`], 10, 10);
  await settle(30);
  // The system shows a copy cursor, and the file stays in the other app's window.
  expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/todo.txt` })).toBe(true);
  expect(await invoke("path_exists", { path: `${HOME}/Desktop/todo.txt` })).toBe(true);
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

test("a drop is hit-tested where the pointer is, also on a 2× display", async () => {
  await goTo(`${HOME}/D`);
  const seen: [number, number][] = [];
  const realRatio = window.devicePixelRatio;
  Object.defineProperty(window, "devicePixelRatio", { value: 2, configurable: true });
  document.elementFromPoint = (x: number, y: number) => {
    seen.push([x, y]);
    return pointAt;
  };
  try {
    pointAt = item("Inbox");
    // The webview reports drop positions in CSS pixels on macOS and Linux.
    mockDrop([`${HOME}/Desktop/todo.txt`], 120, 80);
    await settle(30);
    expect(seen.length).toBeGreaterThan(0);
    expect(seen.every(([x, y]) => x === 120 && y === 80)).toBe(true);
  } finally {
    Object.defineProperty(window, "devicePixelRatio", { value: realRatio, configurable: true });
    document.elementFromPoint = () => pointAt;
  }
});

test("web content from a browser is not taken for a file", async () => {
  await goTo(`${HOME}/D`);
  pointAt = item("Inbox");
  // Linux hands over a link as its URL; macOS hands over no paths at all.
  mockDrop(["https://example.com/cat.png"], 10, 10);
  await settle(30);
  expect(toasts()).toContain("Only files and folders can be dropped here");
  await key("Escape");
  pointAt = item("Inbox");
  mockDrop([], 10, 10);
  await settle(30);
  expect(toasts()).toContain("Only files and folders can be dropped here");
  // A link next to a real file: the file is still copied.
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/Desktop/notes2.txt`, overwrite: false });
  pointAt = item("Inbox");
  mockDrop(["https://example.com/cat.png", `${HOME}/Desktop/notes2.txt`], 10, 10);
  await settle(30);
  expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/notes2.txt` })).toBe(true);
});

// ----- Dragging out to other apps -----

type DragPlugin = { startDrag: ReturnType<typeof vi.fn> };
const tauriGlobal = window as unknown as { __TAURI__?: { drag?: DragPlugin } };

/** Press on `name`, start a drag inside the window, then move past its left edge. */
async function dragOutOfWindow(name: string): Promise<void> {
  await mouse(hit(name), "mousedown", { clientX: 50, clientY: 60 });
  window.dispatchEvent(new MouseEvent("mousemove", { clientX: 300, clientY: 300, buttons: 1 }));
  await settle(10);
  // WebKit keeps sending moves outside the window while the button is held.
  window.dispatchEvent(new MouseEvent("mousemove", { clientX: -5, clientY: 300, buttons: 1 }));
  await settle(20);
}

test("past the window's edge, macOS hands the files to the system drag", async () => {
  const startDrag = vi.fn(async () => {});
  tauriGlobal.__TAURI__ = { drag: { startDrag } };
  try {
    await goTo(`${HOME}/D`);
    await dragOutOfWindow("four.txt");
    expect(startDrag).toHaveBeenCalledWith({ item: [`${HOME}/D/four.txt`], icon: DRAG_IMAGE, mode: "copy" }, expect.any(Function));
    // The window's own drag is over; nothing moved inside it.
    expect($(".drag-ghost")).toBeNull();
    window.dispatchEvent(new MouseEvent("mouseup", { clientX: -5, clientY: 300 }));
    await settle(20);
    expect(names()).toContain("four.txt");
  } finally {
    delete tauriGlobal.__TAURI__;
  }
});

test("past the window's edge, Linux starts its own file drag", async () => {
  const realAgent = navigator.userAgent;
  Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (X11; Linux x86_64)", configurable: true });
  try {
    await goTo(`${HOME}/D`);
    await dragOutOfWindow("one.txt");
    const drags = (globalThis as unknown as { __mockWindow: { drags: string[][] } }).__mockWindow.drags;
    expect(drags.at(-1)).toEqual([`${HOME}/D/one.txt`]);
    expect($(".drag-ghost")).toBeNull();
  } finally {
    Object.defineProperty(navigator, "userAgent", { value: realAgent, configurable: true });
  }
});

test("a drag out that can't start says so", async () => {
  await goTo(`${HOME}/D`);
  await dragOutOfWindow("two.txt");
  expect(toasts().some((t) => t.startsWith("Could not drag out"))).toBe(true);
  expect(names()).toContain("two.txt");
});

test("a drag that left the window and comes back is still the window's own: it moves", async () => {
  tauriGlobal.__TAURI__ = { drag: { startDrag: vi.fn(async () => {}) } };
  try {
    await goTo(`${HOME}/D`);
    await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/D/back.txt`, overwrite: false });
    await goTo(`${HOME}/D`);
    await dragOutOfWindow("back.txt");
    // The system drag returns and is dropped on a folder of the same disk.
    pointAt = item("Inbox");
    mockDrop([`${HOME}/D/back.txt`], 10, 10);
    await settle(30);
    expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/back.txt` })).toBe(true);
    expect(await invoke("path_exists", { path: `${HOME}/D/back.txt` })).toBe(false);
    // Once that drop is done, the same paths from another app copy again.
    pointAt = $(".view-overlay");
    mockDrop([`${HOME}/D/Inbox/back.txt`], 10, 10);
    await settle(30);
    expect(await invoke("path_exists", { path: `${HOME}/D/back.txt` })).toBe(true);
    expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/back.txt` })).toBe(true);
  } finally {
    delete tauriGlobal.__TAURI__;
  }
});

test("a click after a drag out forgets it: later drops of those files copy", async () => {
  tauriGlobal.__TAURI__ = { drag: { startDrag: vi.fn(async () => {}) } };
  try {
    await goTo(`${HOME}/D`);
    await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/D/gone-out.txt`, overwrite: false });
    await goTo(`${HOME}/D`);
    await dragOutOfWindow("gone-out.txt");
    // The drag ended in another app; the user carries on in the window.
    await mouse($("#view"), "mousedown", { clientX: 5, clientY: 5 });
    await mouse($("#view"), "mouseup", { clientX: 5, clientY: 5 });
    pointAt = item("Inbox");
    mockDrop([`${HOME}/D/gone-out.txt`], 10, 10);
    await settle(30);
    expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/gone-out.txt` })).toBe(true);
    expect(await invoke("path_exists", { path: `${HOME}/D/gone-out.txt` })).toBe(true);
  } finally {
    delete tauriGlobal.__TAURI__;
  }
});

type MockWindow = { drags: string[][]; endDrag(landedHere?: boolean): void; buttonUp: boolean };
const mockWindow = () => (globalThis as unknown as { __mockWindow: MockWindow }).__mockWindow;

/** Run `body` with a Linux user agent: drags out go through start_file_drag. */
async function onLinux(body: () => Promise<void>): Promise<void> {
  const realAgent = navigator.userAgent;
  Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (X11; Linux x86_64)", configurable: true });
  try {
    await body();
  } finally {
    Object.defineProperty(navigator, "userAgent", { value: realAgent, configurable: true });
  }
}

/** A new file in ~/D, shown in the view. */
async function freshFile(name: string): Promise<string> {
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/D/${name}`, overwrite: false });
  await goTo(`${HOME}/D`);
  return `${HOME}/D/${name}`;
}

test("Linux: a drag out that ended in another app is forgotten, so the same file dropped later copies", async () => {
  await onLinux(async () => {
    const path = await freshFile("ended.txt");
    await dragOutOfWindow("ended.txt");
    // GTK's drag-end, reported by the backend once the other app took the drop.
    mockWindow().endDrag();
    pointAt = item("Inbox");
    mockDrop([path], 10, 10);
    await settle(30);
    expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/ended.txt` })).toBe(true);
    expect(await invoke("path_exists", { path })).toBe(true);
  });
});

test("Linux: when the button was already up no drag starts, and nothing is remembered", async () => {
  await onLinux(async () => {
    const path = await freshFile("flicked.txt");
    mockWindow().buttonUp = true;
    try {
      await dragOutOfWindow("flicked.txt");
    } finally {
      mockWindow().buttonUp = false;
    }
    pointAt = item("Inbox");
    mockDrop([path], 10, 10);
    await settle(30);
    expect(await invoke("path_exists", { path })).toBe(true);
    expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/flicked.txt` })).toBe(true);
  });
});

test("Linux: a drag out that comes back still moves, though GTK reports its end before the drop", async () => {
  await onLinux(async () => {
    const path = await freshFile("roundtrip.txt");
    await dragOutOfWindow("roundtrip.txt");
    // In the real app GTK's drag-end reaches the page 1-2 ms before the drop
    // event, saying the drop landed on this window.
    mockWindow().endDrag(true);
    pointAt = item("Inbox");
    mockDrop([path], 10, 10);
    await settle(30);
    expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/roundtrip.txt` })).toBe(true);
    expect(await invoke("path_exists", { path })).toBe(false);
  });
});

type PluginEnd = (result: { result: "Dropped" | "Cancel"; cursorPos: { x: number; y: number } }) => void;

/** Drag `name` out on macOS; returns the plugin's end-of-drag callback. */
async function macDragOut(name: string): Promise<PluginEnd> {
  let onEvent: PluginEnd | undefined;
  tauriGlobal.__TAURI__ = { drag: { startDrag: vi.fn(async (_options: unknown, callback?: PluginEnd) => void (onEvent = callback)) } };
  await dragOutOfWindow(name);
  expect(onEvent).toBeTypeOf("function");
  return onEvent!;
}

test("macOS: a cancelled drag out is forgotten at once", async () => {
  try {
    const path = await freshFile("mac-cancel.txt");
    const ended = await macDragOut("mac-cancel.txt");
    ended({ result: "Cancel", cursorPos: { x: 0, y: 0 } });
    pointAt = item("Inbox");
    mockDrop([path], 10, 10);
    await settle(30);
    expect(await invoke("path_exists", { path })).toBe(true);
    expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/mac-cancel.txt` })).toBe(true);
  } finally {
    delete tauriGlobal.__TAURI__;
  }
});

test("macOS: a drag out dropped back here moves, though the plugin reports its end first", async () => {
  try {
    const path = await freshFile("mac-back.txt");
    const ended = await macDragOut("mac-back.txt");
    ended({ result: "Dropped", cursorPos: { x: 0, y: 0 } });
    pointAt = item("Inbox");
    mockDrop([path], 10, 10);
    await settle(30);
    expect(await invoke("path_exists", { path })).toBe(false);
    expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/mac-back.txt` })).toBe(true);
  } finally {
    delete tauriGlobal.__TAURI__;
  }
});

test("macOS: a drag out dropped in another app is forgotten after a few seconds", async () => {
  try {
    const path = await freshFile("mac-away.txt");
    const ended = await macDragOut("mac-away.txt");
    vi.useFakeTimers({ toFake: ["setTimeout"] });
    ended({ result: "Dropped", cursorPos: { x: 0, y: 0 } });
    vi.advanceTimersByTime(3100);
    vi.useRealTimers();
    pointAt = item("Inbox");
    mockDrop([path], 10, 10);
    await settle(30);
    expect(await invoke("path_exists", { path })).toBe(true);
    expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/mac-away.txt` })).toBe(true);
  } finally {
    vi.useRealTimers();
    delete tauriGlobal.__TAURI__;
  }
});

test("the Google Drive row is not a drop target while disconnected", async () => {
  await goTo(`${HOME}/D`);
  const driveRow = $$(".sidebar-row").find((r) => r.textContent?.includes("Google Drive"))!;
  await dragTo("four.txt", driveRow);
  expect(names()).toContain("four.txt");
  expect(await invoke("path_exists", { path: "/four.txt" })).toBe(false);
});

// "/" contains only itself in the view's drop rules: a disk's root dropped
// from another app goes on to the copy, which asks or refuses (the mock asks
// about a clash; the backend refuses copying a folder into itself).
test("a disk's root dropped from another app is passed on to the copy", async () => {
  await goTo(`${HOME}/D`);
  pointAt = item("Inbox");
  mockDrop(["/"], 10, 10);
  await settle(30);
  expect($(".dialog-layer").textContent).toContain("Replace folder “/”?");
  await respond("Skip");
  expect(await invoke("path_exists", { path: `${HOME}/D/Inbox/Users` })).toBe(false);
});

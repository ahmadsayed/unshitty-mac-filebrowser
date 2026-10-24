// Characterization tests for the keyboard: every shortcut in onGlobalKeyDown
// and every key in onViewKeyDown, driven through the real app.
import { beforeAll, describe, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, closeOverlays, cursorName, dialogHeading, item, key, names, open, prefs, selectedNames, settle, title } from "./app";

beforeAll(async () => {
  await boot();
});

async function home(): Promise<void> {
  await closeOverlays();
  await key("Home", { alt: true });
  await settle(10);
}

describe("global shortcuts", () => {
  test("⌘L opens the path entry, ⌘F opens search", async () => {
    await home();
    await key("l", { mod: true });
    expect($("#path-entry").hidden).toBe(false);
    await key("Escape", { target: $("#path-entry") });
    await key("f", { mod: true });
    expect($("#searchbar").hidden).toBe(false);
    await key("Escape", { target: $("#search-entry") });
    expect($("#searchbar").hidden).toBe(true);
  });

  test("⇧⌘N opens New Folder", async () => {
    await home();
    await key("N", { mod: true, shift: true });
    expect(dialogHeading()).toBe("New Folder");
    await closeOverlays();
  });

  test("⌘D bookmarks the current folder and toggles it off", async () => {
    await home();
    // Music is a built-in place and can't be bookmarked; use a plain folder.
    await open("Empty folder");
    const bookmarks = () => $("#sidebar-bookmarks").textContent ?? "";
    await key("d", { mod: true });
    expect(bookmarks()).toContain("Empty folder");
    await key("d", { mod: true });
    expect(bookmarks()).not.toContain("Empty folder");
  });

  test("⇧⌘. and Ctrl+H toggle hidden files", async () => {
    await home();
    expect(names()).not.toContain(".zshrc");
    await key(".", { mod: true, shift: true });
    expect(names()).toContain(".zshrc");
    await key("h", { ctrl: true });
    expect(names()).not.toContain(".zshrc");
  });

  test("⌘R and F5 reload the folder", async () => {
    await home();
    await invoke("make_dir", { path: "/Users/me/Made Elsewhere" });
    expect(names()).not.toContain("Made Elsewhere");
    await key("r", { mod: true });
    expect(names()).toContain("Made Elsewhere");
    await invoke("rename_entry", { from: "/Users/me/Made Elsewhere", to: "/Users/me/Renamed Elsewhere" });
    await key("F5");
    expect(names()).toContain("Renamed Elsewhere");
  });

  test("⌘? shows the keyboard shortcuts", async () => {
    await home();
    await key("?", { mod: true });
    expect(dialogHeading()).toMatch(/Shortcuts/);
    await closeOverlays();
  });

  test("⌘1 / ⌘2 switch list and grid view", async () => {
    await home();
    await key("1", { mod: true });
    expect($$("#view .list-row").length).toBeGreaterThan(0);
    await key("2", { mod: true });
    expect($$("#view .grid-item").length).toBeGreaterThan(0);
  });

  test("⌘= ⌘- ⌘0 zoom the current view", async () => {
    await home();
    await key("2", { mod: true });
    const start = prefs().gridZoom as number;
    await key("=", { mod: true });
    expect(prefs().gridZoom).toBe(start + 1);
    await key("-", { mod: true });
    await key("-", { mod: true });
    expect(prefs().gridZoom).toBe(start - 1);
    await key("0", { mod: true });
    expect(prefs().gridZoom).toBe(2);
  });

  test("Alt+arrows, Alt+Home and ⌘[ ⌘] ⌘↑ navigate", async () => {
    await home();
    await open("Documents");
    expect(title()).toBe("Documents");
    await key("ArrowUp", { alt: true });
    expect(title()).toBe("Home");
    await key("ArrowLeft", { alt: true });
    expect(title()).toBe("Documents");
    await key("ArrowRight", { alt: true });
    expect(title()).toBe("Home");
    await key("[", { mod: true });
    expect(title()).toBe("Documents");
    await key("]", { mod: true });
    expect(title()).toBe("Home");
    await key("[", { mod: true });
    await key("ArrowUp", { mod: true });
    expect(title()).toBe("Home");
    await open("Music");
    await key("Home", { alt: true });
    expect(title()).toBe("Home");
  });

  test("⌘A selects everything; not while typing in a field", async () => {
    await home();
    await key("a", { mod: true });
    expect(selectedNames().length).toBe(names().length);
    await key("Escape");
    await key("f", { mod: true });
    await key("a", { mod: true, target: $("#search-entry") });
    expect(selectedNames()).toEqual([]);
    await key("Escape", { target: $("#search-entry") });
  });

  test("⌘C/⌘V copies and ⌘X/⌘V moves the selection", async () => {
    await home();
    await open("Desktop");
    await key("Home");
    expect(selectedNames()).toEqual(["todo.txt"]);
    await key("c", { mod: true });
    await key("ArrowLeft", { alt: true });
    await open("Public");
    await key("v", { mod: true });
    await settle(20);
    expect(names()).toContain("todo.txt");
    await key("Home");
    await key("x", { mod: true });
    await key("ArrowLeft", { alt: true });
    await open("Empty folder");
    await key("v", { mod: true });
    await settle(20);
    expect(names()).toContain("todo.txt");
    await key("ArrowLeft", { alt: true });
    await open("Public");
    expect(names()).not.toContain("todo.txt");
  });

  test("⌘I shows Properties for one selected item", async () => {
    await home();
    await key("Home");
    await key("i", { mod: true });
    await settle(10);
    expect(dialogHeading()).toBe("Properties");
    await closeOverlays();
  });

  test("⌘Z undoes the last Move to Trash", async () => {
    await home();
    await open("Downloads");
    await key("Home");
    const first = selectedNames()[0];
    await key("Delete");
    await settle(20);
    expect(names()).not.toContain(first);
    await key("z", { mod: true });
    await settle(20);
    expect(names()).toContain(first);
  });

  test("shortcuts are ignored while a dialog is open", async () => {
    await home();
    await key("N", { mod: true, shift: true });
    await key("1", { mod: true, target: document.body });
    expect(dialogHeading()).toBe("New Folder");
    await closeOverlays();
  });
});

describe("view keys", () => {
  test("arrows, Home, End and Page keys move the cursor in list view", async () => {
    await home();
    await key("1", { mod: true });
    await key("Home");
    const all = names();
    expect(cursorName()).toBe(all[0]);
    await key("ArrowDown");
    expect(cursorName()).toBe(all[1]);
    await key("ArrowUp");
    expect(cursorName()).toBe(all[0]);
    await key("End");
    expect(cursorName()).toBe(all[all.length - 1]);
    await key("PageUp");
    expect(cursorName()).toBe(all[0]);
    await key("PageDown");
    expect(cursorName()).toBe(all[all.length - 1]);
    // Left/right do nothing in list view.
    await key("ArrowLeft");
    expect(cursorName()).toBe(all[all.length - 1]);
  });

  test("arrows move by item and row in grid view; Shift extends the selection", async () => {
    await home();
    await key("2", { mod: true });
    await key("Home");
    const all = names();
    await key("ArrowRight");
    expect(cursorName()).toBe(all[1]);
    await key("ArrowLeft");
    expect(cursorName()).toBe(all[0]);
    await key("ArrowRight", { shift: true });
    expect(selectedNames()).toEqual([all[0], all[1]]);
    await key("ArrowDown");
    expect(names().indexOf(cursorName()!)).toBeGreaterThan(1);
  });

  test("⌘↓ and Enter open the selection", async () => {
    await home();
    await key("1", { mod: true });
    await key("Home");
    const first = cursorName();
    await key("ArrowDown", { mod: true });
    expect(title()).toBe(first);
    await key("ArrowLeft", { alt: true });
    await key("Home");
    await key("Enter");
    expect(title()).toBe(first);
  });

  test("F2 renames the selected item", async () => {
    await home();
    await open("Desktop");
    await key("Home");
    await key("F2");
    expect(document.querySelector(".rename-popover")).not.toBeNull();
    await closeOverlays();
  });

  test("Escape clears the selection, or closes search first", async () => {
    await home();
    await key("Home");
    expect(selectedNames().length).toBe(1);
    await key("Escape");
    expect(selectedNames()).toEqual([]);
  });

  test("Backspace goes back; ⌘Backspace moves to the Trash", async () => {
    await home();
    await open("Movies");
    await key("Backspace");
    expect(title()).toBe("Home");
    await open("Movies");
    await key("Home");
    await key("Backspace", { mod: true });
    await settle(20);
    expect(names()).toEqual([]);
  });

  test("⌘Space toggles the item under the cursor", async () => {
    await home();
    await key("Home");
    const first = cursorName();
    await key(" ", { mod: true });
    expect(selectedNames()).not.toContain(first);
    await key(" ", { mod: true });
    expect(selectedNames()).toContain(first);
  });

  test("typing a letter starts a search with it", async () => {
    await home();
    await key("D");
    expect($("#searchbar").hidden).toBe(false);
    expect($<HTMLInputElement>("#search-entry").value).toBe("D");
    expect(names().every((n) => n.toLowerCase().includes("d"))).toBe(true);
    await key("Escape");
    expect($("#searchbar").hidden).toBe(true);
  });

  test("keys with Alt are left to the global shortcuts", async () => {
    await home();
    await key("1", { mod: true });
    await key("Home");
    const before = cursorName();
    await key("End", { alt: false });
    await key("Home", { alt: true });
    expect(title()).toBe("Home");
    expect(cursorName()).not.toBe(before);
    item(names()[0]);
  });
});

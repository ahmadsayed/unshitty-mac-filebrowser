// Clicking, modifier clicks, rubber-band selection, the star button, wheel
// zoom and the mouse back/forward buttons.
import { beforeAll, expect, test } from "vitest";
import { $, boot, goTo, item, key, mouse, names, prefs, selectedNames, settle, title, hit } from "./app";

const LIST_HEADER_H = 35;

beforeAll(async () => {
  await boot();
  await key("1", { mod: true }); // list view: rows are 44 px under a 35 px header
});

async function press(name: string, init: MouseEventInit = {}): Promise<void> {
  await mouse(hit(name), "mousedown", init);
  window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, ...init }));
  hit(name).dispatchEvent(new MouseEvent("mouseup", { bubbles: true, ...init }));
  await settle();
}

test("click selects one item; ⌘-click toggles; ⇧-click selects a range", async () => {
  await goTo("/Users/me/Documents");
  const all = names();
  await press(all[0]);
  expect(selectedNames()).toEqual([all[0]]);
  await press(all[2], { metaKey: true });
  expect(selectedNames()).toEqual([all[0], all[2]]);
  await press(all[2], { metaKey: true });
  expect(selectedNames()).toEqual([all[0]]);
  // The range starts at the last ⌘-clicked item (the anchor), like GNOME Files.
  await press(all[3], { shiftKey: true });
  expect(selectedNames()).toEqual(all.slice(2, 4));
  await press(all[0], { shiftKey: true, metaKey: true });
  expect(selectedNames()).toEqual(all.slice(0, 4));
  // Clicking inside a multi-selection narrows it on release.
  await press(all[1]);
  expect(selectedNames()).toEqual([all[1]]);
});

test("pressing on empty space clears the selection; ⌘ keeps it", async () => {
  await goTo("/Users/me/Documents");
  await press(names()[0]);
  await mouse($("#view"), "mousedown", { clientX: 900, clientY: 600, metaKey: true });
  window.dispatchEvent(new MouseEvent("mouseup"));
  expect(selectedNames().length).toBe(1);
  await mouse($("#view"), "mousedown", { clientX: 900, clientY: 600 });
  window.dispatchEvent(new MouseEvent("mouseup"));
  await settle();
  expect(selectedNames()).toEqual([]);
});

test("dragging on empty space draws a rubber band that selects by layout", async () => {
  await goTo("/Users/me/Documents");
  const all = names();
  await mouse($("#view"), "mousedown", { clientX: 900, clientY: LIST_HEADER_H + 2 });
  // Too small a move doesn't start the band.
  window.dispatchEvent(new MouseEvent("mousemove", { clientX: 901, clientY: LIST_HEADER_H + 3 }));
  expect($("#rubberband").hidden).toBe(true);
  window.dispatchEvent(new MouseEvent("mousemove", { clientX: 10, clientY: LIST_HEADER_H + 100 }));
  await settle();
  expect($("#rubberband").hidden).toBe(false);
  expect(selectedNames()).toEqual(all.slice(0, 3));
  // Past the bottom edge, the band scrolls the view.
  window.dispatchEvent(new MouseEvent("mousemove", { clientX: 10, clientY: 695 }));
  window.dispatchEvent(new MouseEvent("mouseup"));
  await settle();
  expect($("#rubberband").hidden).toBe(true);
});

test("a ⇧ rubber band adds to the selection", async () => {
  await goTo("/Users/me/Documents");
  const all = names();
  await press(all[all.length - 1]);
  await mouse($("#view"), "mousedown", { clientX: 900, clientY: LIST_HEADER_H + 2, shiftKey: true });
  window.dispatchEvent(new MouseEvent("mousemove", { clientX: 10, clientY: LIST_HEADER_H + 30 }));
  window.dispatchEvent(new MouseEvent("mouseup"));
  await settle();
  expect(selectedNames()).toEqual([all[0], all[all.length - 1]]);
});

test("the star button stars and unstars without selecting", async () => {
  await goTo("/Users/me");
  const star = () => item("main.rs").querySelector<HTMLElement>(".star")!;
  expect(star().classList.contains("starred")).toBe(false);
  await mouse(star(), "mousedown");
  await settle(10);
  expect(star().classList.contains("starred")).toBe(true);
  await mouse(star(), "mousedown");
  await settle(10);
  expect(star().classList.contains("starred")).toBe(false);
  // Double-clicking the star doesn't open the file.
  await mouse(star(), "dblclick");
  expect(title()).toBe("Home");
});

test("a right press selects the item under it unless it is already selected", async () => {
  await goTo("/Users/me/Documents");
  const all = names();
  await press(all[0]);
  await press(all[1], { metaKey: true });
  await mouse(hit(all[1]), "mousedown", { button: 2 });
  expect(selectedNames()).toEqual([all[0], all[1]]);
  await mouse(hit(all[3]), "mousedown", { button: 2 });
  item(all[3]).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 }));
  await settle();
  expect(selectedNames()).toEqual([all[3]]);
  await key("Escape", { target: document.body });
});

test("⌘ + wheel zooms by notch or by accumulated small deltas; plain wheel scrolls", async () => {
  await goTo("/Users/me");
  await key("2", { mod: true });
  await key("0", { mod: true });
  const view = $("#view");
  const wheel = (init: WheelEventInit) => view.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, ...init }));
  wheel({ deltaY: -120 });
  expect(prefs().gridZoom).toBe(2);
  wheel({ deltaY: -120, metaKey: true });
  expect(prefs().gridZoom).toBe(3);
  wheel({ deltaY: 3, deltaMode: 1, metaKey: true }); // 3 lines = 48 px, one notch
  expect(prefs().gridZoom).toBe(2);
  wheel({ deltaY: 0, ctrlKey: true });
  for (let i = 0; i < 4; i++) wheel({ deltaY: -5, ctrlKey: true }); // trackpad: small steps add up…
  expect(prefs().gridZoom).toBe(2);
  wheel({ deltaY: -5, ctrlKey: true }); // …to one 24 px step
  expect(prefs().gridZoom).toBe(3);
  wheel({ deltaY: 1, deltaMode: 2, metaKey: true }); // a page
  expect(prefs().gridZoom).toBe(2);
  await key("1", { mod: true });
});

test("mouse back and forward buttons navigate", async () => {
  await goTo("/Users/me");
  await goTo("/Users/me/Music");
  window.dispatchEvent(new MouseEvent("mouseup", { button: 3 }));
  await settle(10);
  expect(title()).toBe("Home");
  window.dispatchEvent(new MouseEvent("mouseup", { button: 4 }));
  await settle(10);
  expect(title()).toBe("Music");
});

test("double-clicking a folder opens it, a file opens it in its app", async () => {
  await goTo("/Users/me");
  await mouse(hit("Music"), "dblclick");
  await settle(10);
  expect(title()).toBe("Music");
  await mouse(hit("Track 1.flac"), "dblclick", { shiftKey: true });
  expect(title()).toBe("Music");
});

test("a grid item takes presses on its icon and name; the rest of its tile starts a rubber band", async () => {
  await goTo("/Users/me");
  await key("2", { mod: true });
  await settle(10);
  const all = names();
  const tile = item(all[0]);

  await mouse(tile.querySelector(".icon")!, "mousedown");
  window.dispatchEvent(new MouseEvent("mouseup"));
  await settle();
  expect(selectedNames()).toEqual([all[0]]);
  await mouse(tile.querySelector(".name")!, "mousedown");
  window.dispatchEvent(new MouseEvent("mouseup"));
  await settle();
  expect(selectedNames()).toEqual([all[0]]);

  // A press on the tile's blank area clears the selection and drags a band.
  await mouse(tile, "mousedown", { clientX: 10, clientY: 10 });
  expect(selectedNames()).toEqual([]);
  window.dispatchEvent(new MouseEvent("mousemove", { clientX: 40, clientY: 40 }));
  await settle();
  expect($("#rubberband").hidden).toBe(false);
  window.dispatchEvent(new MouseEvent("mouseup"));
  await settle();
  expect($("#rubberband").hidden).toBe(true);
  await key("1", { mod: true });
});

// The theme choice in the main menu: Automatic (macOS on a Mac, GNOME
// elsewhere), GNOME or macOS. Switching restyles the window at once and lays
// the view out again with the theme's row sizes, keeping folder, selection
// and scroll position. jsdom applies no CSS, so these check the attribute,
// the menu, the saved pref and the JavaScript layout; the look itself is
// checked with screenshots.
import { beforeAll, expect, test } from "vitest";
import { $, $$, boot, chooseMenu, click, closeOverlays, goTo, item, key, menuLabels, prefs, selectedNames, settle, title } from "./app";

const theme = () => document.documentElement.dataset.theme;
const iconSize = () => document.documentElement.style.getPropertyValue("--icon-size");
const openMainMenu = () => click($("#btn-main-menu"));
const top = (el: HTMLElement) => parseFloat(el.style.top);
const rows = () => $$("#view .item[data-index]").sort((a, b) => Number(a.dataset.index) - Number(b.dataset.index));

beforeAll(boot);

test("Automatic picks macOS on a Mac, and a fresh start shows the grid", () => {
  expect(theme()).toBe("macos");
  expect(prefs().theme ?? "auto").toBe("auto");
  // The theme changes the look only: the grid stays the default view.
  expect($$("#view .grid-item").length).toBeGreaterThan(0);
  expect($$("#view .list-row")).toHaveLength(0);
  // Finder-sized icons: 64 px in the macOS theme.
  expect(iconSize()).toBe("64px");
});

test("the main menu shows the theme choice, with Automatic checked", async () => {
  await openMainMenu();
  const labels = menuLabels();
  const at = (label: string) => labels.indexOf(label);
  expect(at("Show Thumbnails")).toBeLessThan(at("Automatic"));
  expect(labels.slice(at("Automatic"), at("Automatic") + 3)).toEqual(["Automatic", "GNOME", "macOS"]);
  expect(at("macOS")).toBeLessThan(at("Keyboard Shortcuts"));
  expect($$(".popover .menu-heading").map((h) => h.textContent)).toContain("Theme");
  const radio = (label: string) => $$(".popover .menu-item").find((el) => el.textContent?.trim() === label)!;
  expect(radio("Automatic").querySelector(".radio-dot")).not.toBeNull();
  expect(radio("GNOME").querySelector(".radio-dot")).toBeNull();
  expect(radio("macOS").querySelector(".radio-dot")).toBeNull();
  await closeOverlays();
});

test("choosing GNOME switches at once, is saved and keeps the folder and selection", async () => {
  await click(item("Documents").querySelector(".icon")!);
  const before = { title: title(), selected: selectedNames() };
  await openMainMenu();
  await chooseMenu("GNOME");
  expect(theme()).toBe("gnome");
  expect(prefs().theme).toBe("gnome");
  expect(title()).toBe(before.title);
  expect(selectedNames()).toEqual(before.selected);
});

test("list rows follow the theme's row height", async () => {
  await key("1", { mod: true });
  const pitch = () => top(rows()[1]) - top(rows()[0]);
  // 32 px icons: GNOME pads them by 12, macOS by 8.
  expect(pitch()).toBe(44);
  await openMainMenu();
  await chooseMenu("macOS");
  expect(theme()).toBe("macos");
  expect(pitch()).toBe(40);
});

test("switching keeps the first visible item at the top", async () => {
  await goTo("/big");
  const scroller = $("#view-scroll");
  scroller.scrollTop = 5000;
  scroller.dispatchEvent(new Event("scroll"));
  // The view renders on the next animation frame.
  await new Promise((r) => setTimeout(r, 50));
  await settle();
  // macOS: a 28 px list header above 40 px rows.
  const first = rows().find((el) => top(el) >= 5000 - 28)!;
  const index = first.dataset.index;
  await openMainMenu();
  await chooseMenu("GNOME");
  // GNOME: a 35 px header; the same item is now exactly at the top.
  const same = rows().find((el) => el.dataset.index === index)!;
  expect(top(same)).toBe(scroller.scrollTop - 35);
  expect(top(same)).toBe(Number(index) * 44);
});

test("grid positions are the same in both themes", async () => {
  await goTo("/Users/me");
  await key("2", { mod: true });
  const where = () => ({ left: item("Documents").style.left, top: item("Documents").style.top });
  const before = where();
  await openMainMenu();
  await chooseMenu("macOS");
  expect(where()).toEqual(before);
});

test("a storage event from another window switches this one", async () => {
  window.dispatchEvent(new StorageEvent("storage", { key: "prefs", newValue: JSON.stringify({ theme: "gnome" }) }));
  await settle();
  expect(theme()).toBe("gnome");
  await openMainMenu();
  const checked = $$(".popover .menu-item").find((el) => el.querySelector(".radio-dot"));
  expect(checked?.textContent?.trim()).toBe("GNOME");
  await closeOverlays();
});

test("storage events that don't change the theme leave the window alone", async () => {
  await openMainMenu();
  window.dispatchEvent(new StorageEvent("storage", { key: "other", newValue: "{}" }));
  window.dispatchEvent(new StorageEvent("storage", { key: "prefs", newValue: JSON.stringify({ theme: "gnome", view: "list" }) }));
  await settle();
  expect($(".popover-layer")).not.toBeNull();
  expect(theme()).toBe("gnome");
  await closeOverlays();
});

test("switching keeps the zoom; ⌘0 goes back to the theme's icon size", async () => {
  await goTo("/Users/me");
  await key("2", { mod: true });
  await openMainMenu();
  await chooseMenu("GNOME");
  // The zoom the user had stays: switching restyles, it doesn't resize.
  expect(iconSize()).toBe("64px");
  await key("0", { mod: true });
  expect(iconSize()).toBe("96px");
  expect(prefs().gridZoom).toBe(2);
  await openMainMenu();
  await chooseMenu("macOS");
  expect(iconSize()).toBe("96px");
  await key("0", { mod: true });
  expect(iconSize()).toBe("64px");
  expect(prefs().gridZoom).toBe(1);
});

test("Automatic follows the platform again", async () => {
  await openMainMenu();
  await chooseMenu("Automatic");
  expect(theme()).toBe("macos");
  expect(prefs().theme).toBe("auto");
});

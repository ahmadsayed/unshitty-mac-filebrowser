// The theme follows the platform: macOS on a Mac (this file, the test
// setup's Mac user agent), GNOME on Linux (theme-linux.test.ts). There is no
// choice in the main menu. jsdom applies no CSS, so these check the
// attribute, the menu and the JavaScript layout; the look itself is checked
// with screenshots.
import { beforeAll, expect, test } from "vitest";
import { $, $$, boot, click, closeOverlays, key, menuLabels, prefs } from "./app";

const theme = () => document.documentElement.dataset.theme;
const iconSize = () => document.documentElement.style.getPropertyValue("--icon-size");
const top = (el: HTMLElement) => parseFloat(el.style.top);
const rows = () => $$("#view .item[data-index]").sort((a, b) => Number(a.dataset.index) - Number(b.dataset.index));

beforeAll(boot);

test("a Mac shows the macOS theme, and a fresh start shows the grid", () => {
  expect(theme()).toBe("macos");
  expect(document.getElementById("icon-sprite")!.dataset.theme).toBe("macos");
  // The theme changes the look only: the grid stays the default view.
  expect($$("#view .grid-item").length).toBeGreaterThan(0);
  expect($$("#view .list-row")).toHaveLength(0);
  // Finder-sized icons: 64 px in the macOS theme.
  expect(iconSize()).toBe("64px");
});

test("the main menu has no theme choice", async () => {
  await click($("#btn-main-menu"));
  const labels = menuLabels();
  for (const label of ["Automatic", "GNOME", "macOS"]) expect(labels).not.toContain(label);
  expect($$(".popover .menu-heading").map((h) => h.textContent)).not.toContain("Theme");
  // Thumbnails and the shortcuts are one separator apart, as before the choice.
  expect(labels.indexOf("Keyboard Shortcuts")).toBe(labels.indexOf("Show Thumbnails") + 1);
  await closeOverlays();
});

test("list rows use the macOS row height", async () => {
  await key("1", { mod: true });
  // 32 px icons padded by 8.
  expect(top(rows()[1]) - top(rows()[0])).toBe(40);
  await key("2", { mod: true });
});

test("nothing about the theme is saved", async () => {
  await key("1", { mod: true });
  expect(prefs().view).toBe("list");
  expect(prefs()).not.toHaveProperty("theme");
});

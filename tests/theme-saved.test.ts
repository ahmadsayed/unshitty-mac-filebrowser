// A saved theme choice applies from the start, and the choice is read
// defensively: a garbled pref means Automatic, and a storage that refuses
// writes still lets the menu switch the window.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { resolveTheme, themeChoice } from "../src/theme";
import { $, boot, chooseMenu, click } from "./app";

beforeAll(async () => {
  // A saved theme and a zoom the user chose (48 px icons).
  localStorage.setItem("prefs", JSON.stringify({ theme: "gnome", gridZoom: 0 }));
  await boot();
});

afterEach(() => vi.restoreAllMocks());

test("a saved choice and a chosen zoom survive a restart", () => {
  expect(document.documentElement.dataset.theme).toBe("gnome");
  expect(document.documentElement.style.getPropertyValue("--icon-size")).toBe("48px");
});

test("anything but gnome or macos counts as Automatic", () => {
  expect(themeChoice("solarized")).toBe("auto");
  expect(resolveTheme("solarized", true)).toBe("macos");
  expect(resolveTheme(3, false)).toBe("gnome");
  expect(resolveTheme(undefined, true)).toBe("macos");
  expect(resolveTheme("gnome", true)).toBe("gnome");
  expect(resolveTheme("macos", false)).toBe("macos");
});

test("the menu still switches the window when the choice can't be saved", async () => {
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new Error("denied");
  });
  await click($("#btn-main-menu"));
  await chooseMenu("macOS");
  expect(document.documentElement.dataset.theme).toBe("macos");
});

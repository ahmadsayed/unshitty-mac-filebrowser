// Older versions saved a theme choice. It is ignored now (the platform picks
// the theme) and dropped the next time the preferences are saved, while the
// rest of the saved preferences still apply.
import { beforeAll, expect, test } from "vitest";
import { boot, key, prefs } from "./app";

beforeAll(async () => {
  // A GNOME choice saved on a Mac, and a zoom the user chose (48 px icons).
  localStorage.setItem("prefs", JSON.stringify({ theme: "gnome", gridZoom: 0 }));
  await boot();
});

test("an old saved theme is ignored; the chosen zoom survives", () => {
  expect(document.documentElement.dataset.theme).toBe("macos");
  expect(document.documentElement.style.getPropertyValue("--icon-size")).toBe("48px");
});

test("the old choice is dropped when the preferences are saved", async () => {
  await key("1", { mod: true });
  expect(prefs().view).toBe("list");
  expect(prefs()).not.toHaveProperty("theme");
  expect(prefs().gridZoom).toBe(0);
});

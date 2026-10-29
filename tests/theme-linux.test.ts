// Automatic on Linux: the GNOME theme.
import { beforeAll, expect, test } from "vitest";
import { boot } from "./app";

beforeAll(async () => {
  Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (X11; Linux x86_64)", configurable: true });
  await boot();
});

test("Automatic picks GNOME on Linux, and a fresh start shows the grid", () => {
  expect(document.documentElement.dataset.theme).toBe("gnome");
  expect(document.querySelectorAll("#view .grid-item").length).toBeGreaterThan(0);
  expect(document.querySelectorAll("#view .list-row")).toHaveLength(0);
  expect(document.documentElement.style.getPropertyValue("--icon-size")).toBe("96px");
});

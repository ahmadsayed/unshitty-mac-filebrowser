// Automatic on Linux: the GNOME theme.
import { beforeAll, expect, test } from "vitest";
import { boot } from "./app";

beforeAll(async () => {
  Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (X11; Linux x86_64)", configurable: true });
  await boot();
});

test("Automatic picks GNOME on Linux", () => {
  expect(document.documentElement.dataset.theme).toBe("gnome");
});

// Linux: the GNOME theme, with its own row sizes, icon size and icons.
import { beforeAll, expect, test } from "vitest";
import { $$, boot, key } from "./app";

beforeAll(async () => {
  Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (X11; Linux x86_64)", configurable: true });
  await boot();
});

const iconSize = () => document.documentElement.style.getPropertyValue("--icon-size");
const top = (el: HTMLElement) => parseFloat(el.style.top);
const rows = () => $$("#view .item[data-index]").sort((a, b) => Number(a.dataset.index) - Number(b.dataset.index));

test("Linux shows the GNOME theme, and a fresh start shows the grid", () => {
  expect(document.documentElement.dataset.theme).toBe("gnome");
  expect(document.getElementById("icon-sprite")!.dataset.theme).toBe("gnome");
  expect(document.querySelectorAll("#view .grid-item").length).toBeGreaterThan(0);
  expect(document.querySelectorAll("#view .list-row")).toHaveLength(0);
  expect(iconSize()).toBe("96px");
});

// `key` sends ⌘ for `mod` (the helper assumes a Mac), so Ctrl is explicit here.
test("list rows use the GNOME row height; Ctrl+0 goes back to 96 px icons", async () => {
  await key("1", { ctrl: true });
  // 32 px icons padded by 12.
  expect(top(rows()[1]) - top(rows()[0])).toBe(44);
  await key("2", { ctrl: true });
  await key("-", { ctrl: true });
  expect(iconSize()).toBe("64px");
  await key("0", { ctrl: true });
  expect(iconSize()).toBe("96px");
});

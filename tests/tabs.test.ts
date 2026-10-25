// Tabs: independent locations, switching, closing and new tabs from the
// keyboard. Each test file boots a fresh app.
import { beforeAll, expect, test } from "vitest";
import { $, $$, boot, click, goTo, key, names, settle, title } from "./app";

const HOME = "/Users/me";

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

test("⌘T opens a second tab at the current folder", async () => {
  await goTo(HOME);
  expect($("#tabbar").hidden).toBe(true);

  await key("t", { mod: true });
  await settle(10);
  expect($("#tabbar").hidden).toBe(false);
  expect($$(".tab")).toHaveLength(2);
  expect($$(".tab")[1].classList.contains("active")).toBe(true);
  expect(title()).toBe("Home");
  expect(names()).toContain("Documents");
});

test("tabs keep their own location", async () => {
  await goTo(`${HOME}/Documents`);
  expect(title()).toBe("Documents");

  await click($$(".tab")[0]);
  await settle(20);
  expect(title()).toBe("Home");
  expect(names()).toContain("Documents");

  await click($$(".tab")[1]);
  await settle(20);
  expect(title()).toBe("Documents");
  expect(names()).toContain("Quarterly report.pdf");

  await click($$(".tab")[0]);
  await settle(20);
  expect(title()).toBe("Home");
});

test("the active tab's close button removes it", async () => {
  await click($$(".tab.active .tab-close")[0]);
  await settle(20);
  expect($$(".tab")).toHaveLength(0);
  expect($("#tabbar").hidden).toBe(true);
  expect(title()).toBe("Documents");
  expect(names()).toContain("Quarterly report.pdf");
});

test("⌘W closes the active tab and Ctrl+Page Down cycles", async () => {
  await key("t", { mod: true });
  await settle(10);
  await goTo(`${HOME}/Pictures`);
  await key("t", { mod: true });
  await settle(10);
  await goTo(`${HOME}/Music`);
  expect($$(".tab")).toHaveLength(3);

  await key("PageUp", { ctrl: true });
  await settle(20);
  expect(title()).toBe("Pictures");

  await key("PageUp", { ctrl: true });
  await settle(20);
  expect(title()).toBe("Documents");

  await key("PageDown", { ctrl: true });
  await settle(20);
  expect(title()).toBe("Pictures");

  await key("w", { mod: true });
  await settle(20);
  expect($$(".tab")).toHaveLength(2);
  expect(title()).toBe("Music");
});

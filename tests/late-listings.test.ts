// Listings that answer late (#154): a reload's answers are dropped once the
// view went somewhere else. The mock holds a command's calls until the test
// releases them, in the order they were made, so each test sets the order.
import { beforeAll, beforeEach, expect, test } from "vitest";
import { $, $$, boot, click, closeOverlays, goTo, key, menuLabels, mouse, names, settle, title } from "./app";

const HOME = "/Users/me";
const VERSIONED = `${HOME}/Documents/Versioned`;

type MockCalls = { hold(cmd: string): void; release(cmd: string): void; reset(): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
const place = (label: string) => $$(".sidebar-row").find((row) => row.textContent?.trim() === label)!;

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

beforeEach(async () => {
  calls().reset();
  await closeOverlays();
});

// ----- reload -----

test("a reload's versioned folder that lands after going elsewhere is dropped", async () => {
  await goTo(VERSIONED);
  calls().hold("list_dir");
  calls().hold("versioned_root");
  // The reload lists Versioned; going Home asks where Home keeps versions, then lists it.
  await key("F5");
  await key("Home", { alt: true });
  // Both listings land; the reload then asks for Versioned's root after Home's question.
  calls().release("list_dir");
  await settle(20);
  // Home's answer lands first and Home shows; the reload's lands after it.
  calls().release("versioned_root");
  await settle(20);
  expect(title()).toBe("Home");
  await rightClick($("#view"));
  expect(menuLabels()).toContain("Select All");
  expect(menuLabels()).not.toContain("Deleted Files…");
});

test("a reload's listing that lands after going elsewhere is dropped", async () => {
  await goTo(`${HOME}/Documents`);
  calls().hold("list_dir");
  await key("F5");
  await click(place("Recent"));
  await settle(20);
  expect(title()).toBe("Recent");
  const recent = names();
  calls().release("list_dir");
  await settle(20);
  expect(title()).toBe("Recent");
  expect(names()).toEqual(recent);
  expect(names()).not.toContain("Versioned");
});

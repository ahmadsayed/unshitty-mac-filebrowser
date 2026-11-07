// Listings that answer late (#154): the latest navigation shows, and a
// reload's answers are dropped once the view went somewhere else. The mock
// holds a command's calls until the test releases them, in the order they
// were made, so each test sets the order.
import { beforeAll, beforeEach, expect, test } from "vitest";
import { $, $$, boot, click, closeOverlays, goTo, key, menuLabels, mouse, names, settle, title, toasts } from "./app";

const HOME = "/Users/me";
const VERSIONED = `${HOME}/Documents/Versioned`;

type MockCalls = { hold(cmd: string): void; release(cmd: string): void; failNext(cmd: string, error: string): void; reset(): void };
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

// ----- navigate: the latest navigation shows -----

test("two quick navigations: the later one shows, whichever lands first", async () => {
  await goTo(HOME);
  calls().hold("list_dir");
  // Documents waits on its listing; Recent doesn't use list_dir and shows at once.
  await click(place("Documents"));
  await click(place("Recent"));
  await settle(20);
  expect(title()).toBe("Recent");
  calls().release("list_dir");
  await settle(20);
  expect(title()).toBe("Recent");
  expect(names()).not.toContain("Versioned");
  expect(place("Recent").classList.contains("selected")).toBe(true);
  // Only Recent went into the history.
  await key("ArrowLeft", { alt: true });
  await settle(20);
  expect(title()).toBe("Home");
  await key("ArrowRight", { alt: true });
  await settle(20);
  expect(title()).toBe("Recent");
});

test("a navigation a later one overtook says nothing when its listing fails", async () => {
  await goTo(HOME);
  calls().hold("list_dir");
  calls().failNext("list_dir", "Failed to read directory: Permission denied (os error 13)");
  await click(place("Documents"));
  await click(place("Recent"));
  calls().release("list_dir");
  await settle(20);
  expect(title()).toBe("Recent");
  expect(toasts().join(" ")).not.toContain("Unable to access");
});

test("a typed path still being checked when a later navigation starts isn't opened", async () => {
  await goTo(HOME);
  calls().hold("list_dir");
  // The path entry lists the typed path before going there; Recent starts meanwhile.
  await goTo(`${HOME}/Documents`);
  await click(place("Recent"));
  await settle(20);
  calls().release("list_dir");
  await settle(20);
  expect(title()).toBe("Recent");
  expect(names()).not.toContain("Versioned");

  // Nor does one whose check fails say so.
  calls().hold("list_dir");
  await goTo(`${HOME}/Nowhere`);
  await click(place("Starred"));
  await settle(20);
  calls().release("list_dir");
  await settle(20);
  expect(title()).toBe("Starred");
  expect(toasts().join(" ")).not.toContain("Unable to find");
});

test("Back that a later navigation overtook leaves the history as it was", async () => {
  await goTo(HOME);
  await goTo(`${HOME}/Documents`);
  calls().hold("list_dir");
  await key("ArrowLeft", { alt: true });
  await click(place("Recent"));
  await settle(20);
  calls().release("list_dir");
  await settle(20);
  expect(title()).toBe("Recent");
  // Back goes to Documents, then Home: the overtaken Back took nothing off the history.
  await key("ArrowLeft", { alt: true });
  await settle(20);
  expect(title()).toBe("Documents");
  await key("ArrowLeft", { alt: true });
  await settle(20);
  expect(title()).toBe("Home");
});

test("going elsewhere while a tab switch lists its folder stays in that tab", async () => {
  await goTo(HOME);
  await key("t", { mod: true });
  await settle(10);
  await goTo(`${HOME}/Documents`);
  await click($$(".tab")[0]);
  await settle(20);
  expect(title()).toBe("Home");

  calls().hold("list_dir");
  await click($$(".tab")[1]);
  await click(place("Recent"));
  await settle(20);
  calls().release("list_dir");
  await settle(20);
  // The switch isn't undone, and Recent isn't replaced by the tab's Documents.
  expect($$(".tab")[1].classList.contains("active")).toBe(true);
  expect(title()).toBe("Recent");
  expect(names()).not.toContain("Versioned");
  await click($$(".tab")[0]);
  await settle(20);
  expect(title()).toBe("Home");
  await key("w", { mod: true });
  await settle(20);
});

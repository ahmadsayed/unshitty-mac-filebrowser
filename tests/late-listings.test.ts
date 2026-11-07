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
  // One tab, even after a failed test.
  for (let i = 0; i < 3 && $$(".tab").length > 1; i++) await key("w", { mod: true });
  await settle(20);
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

test("a reload's failed listing that lands after going elsewhere changes nothing", async () => {
  await goTo(`${HOME}/Documents`);
  calls().hold("list_dir");
  calls().failNext("list_dir", "Failed to read directory: Permission denied (os error 13)");
  await key("F5");
  await click(place("Recent"));
  await settle(20);
  expect(title()).toBe("Recent");
  const recent = names();
  expect(recent.length).toBeGreaterThan(0);
  // Documents' failure isn't shown on Recent: no error page, no empty view.
  calls().release("list_dir");
  await settle(20);
  expect(title()).toBe("Recent");
  expect(names()).toEqual(recent);
});

test("a reload that finds the view went elsewhere while it asked for the shares lists nothing", async () => {
  await click(place("Network"));
  await settle(20);
  expect(title()).toBe("Network");
  calls().hold("list_received");
  await key("F5");
  await click(place("Recent"));
  await settle(20);
  expect(title()).toBe("Recent");
  const listed = calls().count("recent_files");
  calls().release("list_received");
  await settle(20);
  expect(calls().count("recent_files")).toBe(listed);
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

// ----- tabs: a tab switch is a navigation too -----

/** A second tab that came from `history` to `at`, then back to the first tab,
 *  which shows `first`. */
async function secondTab(history: string[], at: string, first: string): Promise<void> {
  await key("t", { mod: true });
  await settle(10);
  for (const path of [...history, at]) await goTo(path);
  await click($$(".tab")[0]);
  await settle(20);
  expect(title()).toBe(first);
}

const tabTitles = () => $$(".tab .tab-title").map((el) => el.textContent);

/** Press Back once and say where it went. */
async function back(): Promise<string> {
  await key("ArrowLeft", { alt: true });
  await settle(20);
  return title();
}

test("going elsewhere while a tab switch lists its folder stays in that tab", async () => {
  await goTo(HOME);
  await secondTab([], `${HOME}/Documents`, "Home");

  calls().hold("list_dir");
  await click($$(".tab")[1]);
  await click(place("Recent"));
  await settle(20);
  // Recent shows in the tab at once.
  expect($$(".tab")[1].classList.contains("active")).toBe(true);
  expect(title()).toBe("Recent");
  calls().release("list_dir");
  await settle(20);
  // The switch isn't undone, and Recent isn't replaced by the tab's Documents.
  expect($$(".tab")[1].classList.contains("active")).toBe(true);
  expect(title()).toBe("Recent");
  expect(names()).not.toContain("Versioned");
  expect(tabTitles()).toEqual(["Home", "Recent"]);
  // Recent went on the tab's own history: Documents, then Home.
  expect(await back()).toBe("Documents");
  expect(await back()).toBe("Home");
  expect($<HTMLButtonElement>("#btn-back").disabled).toBe(true);
  await click($$(".tab")[0]);
  await settle(20);
  expect(title()).toBe("Home");
  await key("w", { mod: true });
  await settle(20);
});

test("a navigation that lands after a tab switch showed goes on that tab's history", async () => {
  // The first tab came from Music to Desktop; the second from Desktop, then Home, to Documents.
  await goTo(`${HOME}/Music`);
  await goTo(`${HOME}/Desktop`);
  await secondTab([HOME], `${HOME}/Documents`, "Desktop");

  // Both wait: the tab's Documents lands first, then Downloads.
  calls().hold("list_dir");
  await click($$(".tab")[1]);
  await click(place("Downloads"));
  await settle(20);
  calls().release("list_dir");
  await settle(20);
  expect(title()).toBe("Downloads");
  expect(tabTitles()).toEqual(["Desktop", "Downloads"]);
  expect(await back()).toBe("Documents");
  expect(await back()).toBe("Home");
  expect(await back()).toBe("Desktop");
  await key("w", { mod: true });
  await settle(20);
});

test("a tab switch shows its folder when the navigation that overtook it fails", async () => {
  await goTo(HOME);
  await secondTab([], `${HOME}/Documents`, "Home");

  calls().hold("list_dir");
  calls().failNext("recent_files", "Permission denied");
  await click($$(".tab")[1]);
  await click(place("Recent"));
  await settle(20);
  calls().release("list_dir");
  await settle(20);
  expect(title()).toBe("Documents");
  expect(names()).toContain("Versioned");
  expect($$(".tab")[1].classList.contains("active")).toBe(true);
  // Each tab still has its own folder.
  await click($$(".tab")[0]);
  await settle(20);
  expect(title()).toBe("Home");
  await click($$(".tab")[1]);
  await settle(20);
  expect(title()).toBe("Documents");
  expect(tabTitles()).toEqual(["Home", "Documents"]);
  expect(await back()).toBe("Home");
  await key("w", { mod: true });
  await settle(20);
});

test("closing a tab shows the next one when the navigation that overtook it fails", async () => {
  await goTo(HOME);
  await secondTab([], `${HOME}/Documents`, "Home");
  await click($$(".tab")[1]);
  await settle(20);
  expect(title()).toBe("Documents");

  calls().hold("list_dir");
  calls().failNext("recent_files", "Permission denied");
  await key("w", { mod: true });
  await click(place("Recent"));
  await settle(20);
  calls().release("list_dir");
  await settle(20);
  expect(title()).toBe("Home");
  expect(names()).not.toContain("Versioned");
  expect($("#tabbar").hidden).toBe(true);
});

test("Back while a tab switch lists its folder steps back in that tab", async () => {
  // The first tab came from Music to Desktop; the second from Desktop, then Home, to Documents.
  await goTo(`${HOME}/Music`);
  await goTo(`${HOME}/Desktop`);
  await secondTab([HOME], `${HOME}/Documents`, "Desktop");

  calls().hold("list_dir");
  await click($$(".tab")[1]);
  await key("ArrowLeft", { alt: true });
  await settle(20);
  calls().release("list_dir");
  await settle(20);
  // Back went to the tab's Home, not to the first tab's Music.
  expect(title()).toBe("Home");
  expect(tabTitles()).toEqual(["Desktop", "Home"]);
  await key("ArrowRight", { alt: true });
  await settle(20);
  expect(title()).toBe("Documents");
  expect(await back()).toBe("Home");
  expect(await back()).toBe("Desktop");
  await key("w", { mod: true });
  await settle(20);
});

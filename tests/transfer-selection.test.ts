// A finished paste selects what it made, unless the user chose something else
// (or went elsewhere) while it ran (#50). The mock backend holds each copy
// until the test releases it (__mockOps), so the user can act in between.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { $, $$, boot, click, closeOverlays, cursorName, goTo, hit, key, mouse, names, selectedNames, settle, title } from "./app";

type MockOps = { hold(): void; held(): number; release(): void; reset(): void };
const ops = () => (globalThis as unknown as { __mockOps: MockOps }).__mockOps;
const HOME = "/Users/me";

beforeAll(async () => {
  await boot();
});

afterEach(async () => {
  ops().release();
  ops().reset();
  await settle(20);
  await closeOverlays();
});

async function select(name: string, add = false): Promise<void> {
  await mouse(hit(name), "mousedown", { button: 0, metaKey: add });
  await mouse(hit(name), "mouseup", { button: 0, metaKey: add });
  await mouse(hit(name), "click", { button: 0, metaKey: add });
}

async function waitHeld(n = 1): Promise<void> {
  await vi.waitFor(() => expect(ops().held()).toBe(n), { timeout: 2000 });
  await settle(5);
}

/** Copy `picks` from `from` and paste them into `to`, with the copy held. */
async function startCopy(from: string, picks: string[], to: string): Promise<void> {
  ops().hold();
  await goTo(from);
  for (const [i, name] of picks.entries()) await select(name, i > 0);
  await key("c", { mod: true });
  await goTo(to);
  await key("v", { mod: true });
  await waitHeld();
}

async function finish(): Promise<void> {
  // Release every queued item, including ones that start after a release.
  for (let i = 0; i < 6; i++) {
    ops().release();
    await settle(10);
  }
  await settle(20);
}

test("a normal paste still selects what it made", async () => {
  await startCopy(`${HOME}/Desktop`, ["todo.txt"], `${HOME}/Public`);
  await finish();
  expect(names()).toContain("todo.txt");
  expect(selectedNames()).toEqual(["todo.txt"]);
  expect(cursorName()).toBe("todo.txt");
});

test("a selection made while the copy runs stays selected, cursor included", async () => {
  await startCopy(`${HOME}/Desktop`, ["todo.txt"], `${HOME}/Documents`);
  await select("Notes.md");
  await finish();
  expect(names()).toContain("todo.txt");
  expect(selectedNames()).toEqual(["Notes.md"]);
  expect(cursorName()).toBe("Notes.md");
  // The anchor stayed too: Shift+click extends from Notes.md.
  await mouse(hit("Budget 2026.xlsx"), "mousedown", { button: 0, shiftKey: true });
  await mouse(hit("Budget 2026.xlsx"), "mouseup", { button: 0, shiftKey: true });
  expect(selectedNames()).toContain("Notes.md");
  expect(selectedNames()).toContain("Budget 2026.xlsx");
});

test("clearing the selection while the copy runs is a choice too", async () => {
  await startCopy(`${HOME}/Desktop`, ["todo.txt"], `${HOME}/Templates`);
  await key("Escape");
  await finish();
  expect(names()).toContain("todo.txt");
  expect(selectedNames()).toEqual([]);
});

test("going away and back doesn't bring back the paste's selection", async () => {
  await startCopy(`${HOME}/Desktop`, ["todo.txt"], `${HOME}/Music`);
  await goTo(`${HOME}/Movies`);
  await goTo(`${HOME}/Music`);
  await finish();
  expect(title()).toBe("Music");
  expect(names()).toContain("todo.txt");
  expect(selectedNames()).toEqual([]);
});

test("a paste finishing while another folder is open selects nothing there", async () => {
  await startCopy(`${HOME}/Desktop`, ["todo.txt"], `${HOME}/Pictures`);
  await goTo(`${HOME}/Movies`);
  await select("Holiday.mov");
  await finish();
  expect(title()).toBe("Movies");
  expect(selectedNames()).toEqual(["Holiday.mov"]);
});

test("a new tab on the same folder doesn't get the paste's selection", async () => {
  await startCopy(`${HOME}/Desktop`, ["todo.txt"], `${HOME}/Downloads`);
  await key("t", { mod: true });
  await settle(20);
  expect(title()).toBe("Downloads");
  await finish();
  expect(names()).toContain("todo.txt");
  expect(selectedNames()).toEqual([]);
  await key("w", { mod: true });
  await settle(20);
});

test("queued pastes each select their own items when left alone", async () => {
  await goTo(`${HOME}/Documents`);
  await select("Notes.md");
  await key("c", { mod: true });
  ops().hold();
  await goTo(`${HOME}/Desktop`);
  await key("v", { mod: true });
  await waitHeld();
  // A second paste waits behind the first.
  await select("todo.txt");
  await key("c", { mod: true });
  await goTo(`${HOME}/Empty folder`);
  await key("v", { mod: true });
  await settle(10);
  await finish();
  expect(title()).toBe("Empty folder");
  expect(selectedNames()).toEqual(["todo.txt"]);
});

test("a queued paste doesn't override a selection made after it was pasted", async () => {
  await goTo(`${HOME}/Documents`);
  await select("Roadmap.key");
  await key("c", { mod: true });
  ops().hold();
  await goTo(`${HOME}/Movies`);
  await key("v", { mod: true });
  await waitHeld();
  await select("Holiday.mov");
  await finish();
  expect(names()).toContain("Roadmap.key");
  expect(selectedNames()).toEqual(["Holiday.mov"]);
});

test("a cancelled paste selects only what it finished, and never a missing file", async () => {
  ops().hold();
  await goTo(`${HOME}/Documents`);
  await select("Quarterly report.pdf");
  await select("Budget 2026.xlsx", true);
  await key("c", { mod: true });
  await goTo(`${HOME}/Pictures/Screenshots`);
  await key("v", { mod: true });
  await waitHeld();
  ops().release();
  await waitHeld();
  // Cancel the second item from the operations popover.
  await click($("#btn-operations"));
  await settle(2);
  await click($$(".operation-row .operation-cancel")[0]);
  await finish();
  const landed = names().filter((n) => n === "Quarterly report.pdf" || n === "Budget 2026.xlsx");
  expect(landed).toHaveLength(1);
  expect(selectedNames()).toEqual(landed);
});

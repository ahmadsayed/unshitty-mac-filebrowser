// Operation progress in the real app: pastes, trash batches and remote opens
// show in the operations popover and can be cancelled. The mock backend holds
// commands until the test releases them (__mockOps).
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, closeOverlays, goTo, item, key, mouse, names, open, settle, toasts, type } from "./app";

type MockOps = {
  hold(): void; held(): number; step(bytes: number): void; release(): void; reset(): void; failMeasure(fail: boolean): void;
  holdMeasure(): void; lateDone(late: boolean): void;
};
const ops = () => (globalThis as unknown as { __mockOps: MockOps }).__mockOps;
const HOME = "/Users/me";

beforeAll(async () => {
  await boot();
});

afterEach(async () => {
  ops().release();
  ops().reset();
  await settle(20);
  // An open popover takes the next keystrokes, as in the app.
  await closeOverlays();
});

async function select(name: string, add = false): Promise<void> {
  await mouse(item(name), "mousedown", { button: 0, metaKey: add });
  await mouse(item(name), "mouseup", { button: 0, metaKey: add });
  await mouse(item(name), "click", { button: 0, metaKey: add });
}

async function waitHeld(n = 1): Promise<void> {
  await vi.waitFor(() => expect(ops().held()).toBe(n), { timeout: 2000 });
  await settle(5);
}

async function showPopover(): Promise<void> {
  if (!document.querySelector(".operations-popover")) await click($("#btn-operations"));
  await settle(2);
}

const rows = () => $$(".operation-row");
const detail = (i = 0) => rows()[i].querySelector(".operation-detail")!.textContent;
const rowTitle = (i = 0) => rows()[i].querySelector(".operation-title")!.textContent;

async function copyInto(from: string, picks: string[], to: string): Promise<void> {
  await goTo(from);
  for (const [i, name] of picks.entries()) await select(name, i > 0);
  await key("c", { mod: true });
  await goTo(to);
  await key("v", { mod: true });
}

test("a slow paste shows the operations button with its progress", async () => {
  ops().hold();
  await copyInto(`${HOME}/Desktop`, ["todo.txt"], `${HOME}/Public`);
  await waitHeld();
  expect($("#btn-operations").hidden).toBe(false);
  await showPopover();
  expect(rowTitle()).toBe("Copying “todo.txt”");
  ops().step(100);
  await settle(2);
  expect(detail()).toBe("100 bytes of 312 bytes");
  ops().release();
  await settle(20);
  expect(names()).toContain("todo.txt");
  expect(rows()).toHaveLength(0);
});

test("Cancel ends a paste without an error toast and keeps finished items", async () => {
  ops().hold();
  await copyInto(`${HOME}/Documents`, ["Notes.md", "Budget 2026.xlsx"], `${HOME}/Templates`);
  await waitHeld();
  ops().release();
  await waitHeld();
  await showPopover();
  await click(rows()[0].querySelector(".operation-cancel")!);
  await settle(20);
  const copied = names();
  expect(copied.filter((n) => n === "Notes.md" || n === "Budget 2026.xlsx")).toHaveLength(1);
  expect(toasts().some((t) => t.startsWith("Could not"))).toBe(false);
});

test("a second paste waits for the first, and cancelling the waiting one removes only it", async () => {
  ops().hold();
  await copyInto(`${HOME}/Desktop`, ["todo.txt"], `${HOME}/Music`);
  await waitHeld();
  await goTo(`${HOME}/Documents`);
  await select("Notes.md");
  await key("c", { mod: true });
  await goTo(`${HOME}/Music`);
  await key("v", { mod: true });
  await settle(10);
  await showPopover();
  expect(rows().map((_, i) => detail(i))).toEqual([expect.any(String), "Waiting…"]);
  await click(rows()[1].querySelector(".operation-cancel")!);
  await settle(5);
  expect(rows()).toHaveLength(1);
  ops().release();
  await settle(20);
  expect(names()).toContain("todo.txt");
  expect(names()).not.toContain("Notes.md");
});

test("a queued paste runs after the first one", async () => {
  ops().hold();
  await copyInto(`${HOME}/Desktop`, ["todo.txt"], `${HOME}/Pictures/Screenshots`);
  await waitHeld();
  await goTo(`${HOME}/Documents`);
  await select("Notes.md");
  await key("c", { mod: true });
  await goTo(`${HOME}/Pictures/Screenshots`);
  await key("v", { mod: true });
  await settle(10);
  ops().release();
  await waitHeld();
  ops().release();
  await settle(20);
  expect(names()).toEqual(expect.arrayContaining(["todo.txt", "Notes.md"]));
});

test("when measuring fails the paste counts files instead", async () => {
  ops().hold();
  ops().failMeasure(true);
  await copyInto(`${HOME}/Desktop`, ["todo.txt"], `${HOME}/Movies`);
  await waitHeld();
  await showPopover();
  expect(detail()).toBe("0 of 1 files");
});

test("Cancel while preparing ends the paste at once, before measuring finishes", async () => {
  ops().holdMeasure();
  await copyInto(`${HOME}/Desktop`, ["todo.txt"], `${HOME}/Empty folder`);
  await waitHeld();
  await showPopover();
  expect(detail()).toBe("Preparing…");
  await click(rows()[0].querySelector(".operation-cancel")!);
  await settle(5);
  expect(rows()).toHaveLength(0);
  ops().release();
  await settle(20);
  expect(names()).not.toContain("todo.txt");
});

test("a trash batch shows progress and Cancel keeps the rest", async () => {
  await goTo(`${HOME}/Documents`);
  ops().hold();
  await select("Notes.md");
  await select("Budget 2026.xlsx", true);
  await key("Delete");
  await waitHeld();
  await showPopover();
  expect(rowTitle()).toBe("Moving 2 files to the Trash");
  await click(rows()[0].querySelector(".operation-cancel")!);
  ops().release();
  await settle(20);
  expect(names().filter((n) => n === "Notes.md" || n === "Budget 2026.xlsx")).toHaveLength(1);
  expect(toasts().some((t) => t.includes("moved to trash"))).toBe(true);
});

test("a finished item counts all its bytes, even when its last update arrives late", async () => {
  ops().hold();
  ops().lateDone(true);
  await copyInto(`${HOME}/Documents`, ["Quarterly report.pdf", "Roadmap.key"], `${HOME}/Pictures`);
  await waitHeld();
  ops().release();
  await waitHeld();
  await showPopover();
  // Quarterly report.pdf (2,480,331 bytes) is done; Roadmap.key (9,810,220)
  // is running.
  expect(detail()).toBe("2.5 MB of 12.3 MB");
});

test("each finished item shows up in the open folder while the rest copy", async () => {
  ops().hold();
  await copyInto(`${HOME}/Documents`, ["Quarterly report.pdf", "Roadmap.key"], `${HOME}/Downloads`);
  await waitHeld();
  ops().release();
  await waitHeld();
  // Reloads are limited to one a second, so wait for it like a person would.
  await vi.waitFor(() => expect(names()).toContain("Quarterly report.pdf"), { timeout: 2000 });
  expect(names()).not.toContain("Roadmap.key");
});

// Last: it leaves the app on a remote server.
test("opening an SSH file shows Opening “name” and can be cancelled", async () => {
  await click($$(".sidebar-row").find((row) => row.textContent?.trim() === "Network")!);
  await settle(10);
  await type($<HTMLInputElement>(".network-link"), "user@host");
  await click($(".network-paste .btn"));
  await settle(30);
  ops().hold();
  await open("notes.txt");
  await waitHeld();
  await showPopover();
  expect(rowTitle()).toBe("Opening “notes.txt”");
  await click(rows()[0].querySelector(".operation-cancel")!);
  await settle(20);
  expect(toasts().some((t) => t.startsWith("Could not open"))).toBe(false);
  expect(rows()).toHaveLength(0);
  await invoke("sftp_disconnect", { id: "user@host" });
});

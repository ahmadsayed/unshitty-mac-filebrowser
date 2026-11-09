// Space steps through an item's versions: an Arbesk asset's saves, and a
// file's history in a folder that keeps versions. The real app against the
// mock, with only the WebGL drawing (showModel) replaced, as jsdom has none.
import { beforeAll, beforeEach, expect, test, vi } from "vitest";
import type { Group } from "three";
import { $, boot, click, goTo, hit, key, settle } from "./app";
import { ARBESK_MOCK_ADDRESS, invoke } from "../dev/mock-tauri/core";

const viewer = vi.hoisted(() => ({ showModel: vi.fn() }));

vi.mock("../src/model-viewer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/model-viewer")>();
  return { ...actual, showModel: viewer.showModel };
});

type MockCalls = {
  count(cmd: string): number;
  failNext(cmd: string, error: string): void;
  hold(cmd: string): void;
  release(cmd: string): void;
};
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;

const UNTITLED = `arbesk://${ARBESK_MOCK_ADDRESS}/2~Untitled%20Collection`;
const VERSIONED = "/Users/me/Documents/Versioned";

function fakeView(host: HTMLElement) {
  const canvas = document.createElement("canvas");
  canvas.className = "preview-model";
  host.appendChild(canvas);
  return { dispose: vi.fn(() => canvas.remove()) };
}

/** Wait for `ok`, checking every 10 ms (three.js loads on the first preview). */
async function until(ok: () => boolean, ms = 10_000): Promise<void> {
  const end = Date.now() + ms;
  while (!ok()) {
    if (Date.now() > end) throw new Error("timed out waiting for the preview");
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

/** Whether the preview shows its content (or why there is none), not "Loading…". */
const shown = () => [...$("#preview-body").children].some((child) => !child.classList.contains("preview-loading"));

async function preview(name: string): Promise<void> {
  await click(hit(name));
  await key(" ");
  await until(shown);
  await settle(10);
}

const control = () => $("#preview-versions");
const title = () => control().querySelector(".preview-version-title")?.textContent;
const older = () => control().querySelector<HTMLButtonElement>("[aria-label='Older Version']")!;
const newer = () => control().querySelector<HTMLButtonElement>("[aria-label='Newer Version']")!;

/** Copies in the last scene the preview drew. */
function copies(): number {
  const scene = viewer.showModel.mock.calls.at(-1)![1] as Group;
  return scene.children.length;
}

async function step(button: HTMLButtonElement): Promise<void> {
  await click(button);
  await until(shown);
  await settle(10);
}

beforeAll(async () => {
  await boot();
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
});

beforeEach(() => {
  viewer.showModel.mockReset();
  viewer.showModel.mockImplementation(fakeView);
});

test("an Arbesk asset with three saves shows Version 3 of 3", async () => {
  await goTo(UNTITLED);
  await preview("Cube.gltf");
  await until(() => !control().hidden);
  expect(title()).toBe("Version 3 of 3");
  expect(newer().disabled).toBe(true);
  expect(older().disabled).toBe(false);
  expect(copies()).toBe(3);
  await key("Escape");
});

test("Older Version shows the older save", async () => {
  await goTo(UNTITLED);
  await preview("Cube.gltf");
  await until(() => !control().hidden);
  await step(older());
  expect(title()).toBe("Version 2 of 3");
  expect(copies()).toBe(2);
  expect([older().disabled, newer().disabled]).toEqual([false, false]);
  await key("Escape");
});

test("the list jumps to any version", async () => {
  await goTo(UNTITLED);
  await preview("Cube.gltf");
  await until(() => !control().hidden);
  await click(control().querySelector<HTMLButtonElement>(".preview-version-label")!);
  const items = [...document.querySelectorAll<HTMLElement>(".popover .menu-item")];
  expect(items.map((i) => i.querySelector(".menu-label")?.textContent ?? i.textContent)).toEqual(
    expect.arrayContaining([expect.stringContaining("Version 3 — Current"), expect.stringContaining("Version 1")]),
  );
  await click(items.find((i) => /Version 1(?!\d)/.test(i.textContent ?? ""))!);
  await until(shown);
  await settle(10);
  expect(title()).toBe("Version 1 of 3");
  expect(copies()).toBe(1);
  expect(older().disabled).toBe(true);
  await key("Escape");
});

test("a quick second step shows only the second version", async () => {
  await goTo(UNTITLED);
  await preview("Cube.gltf");
  await until(() => !control().hidden);
  const before = viewer.showModel.mock.calls.length;
  calls().hold("arbesk_scene");
  await click(older());
  await click(older());
  calls().release("arbesk_scene");
  await until(() => viewer.showModel.mock.calls.length > before);
  await settle(50);
  expect(viewer.showModel.mock.calls.length - before).toBe(1);
  expect(copies()).toBe(1);
  expect(title()).toBe("Version 1 of 3");
  await key("Escape");
});

test("Escape during a version load shows nothing afterwards", async () => {
  await goTo(UNTITLED);
  await preview("Cube.gltf");
  await until(() => !control().hidden);
  const before = viewer.showModel.mock.calls.length;
  calls().hold("arbesk_scene");
  await click(older());
  await key("Escape");
  calls().release("arbesk_scene");
  await settle(50);
  expect($("#preview-layer").hidden).toBe(true);
  expect(viewer.showModel.mock.calls.length).toBe(before);
});

test("a local text file steps to its older version", async () => {
  await goTo(VERSIONED);
  await preview("notes.txt");
  await until(() => !control().hidden);
  // Three versions with content; the deletion marker isn't one.
  expect(title()).toBe("Version 3 of 3");
  expect($("#preview-body pre").textContent).toContain(`Preview of ${VERSIONED}/notes.txt`);
  await step(older());
  expect(title()).toBe("Version 2 of 3");
  expect($("#preview-body pre").textContent).toMatch(/^Preview of \/Users\/me\/\.cache\/opened-versions\//);
  await key("Escape");
});

test("Open still opens the current file", async () => {
  const log = vi.spyOn(console, "log");
  await goTo(VERSIONED);
  await preview("notes.txt");
  await until(() => !control().hidden);
  await step(older());
  const open = [...$("#preview-actions").querySelectorAll("button")].find((b) => b.textContent === "Open")!;
  await click(open);
  await settle(20);
  expect(log).toHaveBeenCalledWith("mock open_file", `${VERSIONED}/notes.txt`);
  log.mockRestore();
  await key("Escape");
});

test("a version that fails says so; the others still show", async () => {
  await goTo(VERSIONED);
  await preview("notes.txt");
  await until(() => !control().hidden);
  calls().failNext("open_version", "Could not read the version.");
  await step(older());
  expect($("#preview-body .preview-empty")?.textContent).toContain("No preview available.");
  expect(title()).toBe("Version 2 of 3");
  await step(newer());
  expect($("#preview-body pre").textContent).toContain(`Preview of ${VERSIONED}/notes.txt`);
  await key("Escape");
});

test("one version, no control", async () => {
  await goTo(VERSIONED);
  await preview("image.png");
  await settle(30);
  expect(control().hidden).toBe(true);
  await key("Escape");
  await goTo("/Users/me/Documents");
  await preview("Notes.md");
  await settle(30);
  expect(control().hidden).toBe(true);
  await key("Escape");
});

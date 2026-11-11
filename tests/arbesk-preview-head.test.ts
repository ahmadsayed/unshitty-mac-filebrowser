// Space on an Arbesk model asks for its scene at once (performance plan,
// phase 1): the newest save as the backend reads it, which answers that
// save's CID; the parts and the files switcher are read by it, and the
// version walk fills the version control alongside. A save that lands
// meanwhile shows in the control and swaps nothing. The real app against
// the mock, with only the WebGL drawing (showModel) replaced.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, boot, click, closeOverlays, goTo, hit, key, settle } from "./app";

const viewer = vi.hoisted(() => ({ showModel: vi.fn() }));

vi.mock("../src/model-viewer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/model-viewer")>();
  return { ...actual, showModel: viewer.showModel };
});

type MockCalls = {
  count(cmd: string): number;
  argsOf(cmd: string): Record<string, unknown>[];
  hold(cmd: string): void;
  release(cmd: string): void;
  failNext(cmd: string, error: string): void;
  reset(): void;
};
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
type MockArbesk = { saveAgain(token: string, asset: string): void };
const mockArbesk = () => (globalThis as unknown as { __mockArbesk: MockArbesk }).__mockArbesk;

const SHAPES = "arbesk://0xabababababababababababababababababababab/1~Shapes";

function fakeView(host: HTMLElement) {
  const canvas = document.createElement("canvas");
  canvas.className = "preview-model";
  host.appendChild(canvas);
  return { dispose: vi.fn(() => canvas.remove()) };
}

async function until(ok: () => boolean, ms = 10_000): Promise<void> {
  const end = Date.now() + ms;
  while (!ok()) {
    if (Date.now() > end) throw new Error("timed out waiting for the preview");
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

const shown = () => [...$("#preview-body").children].some((child) => !child.classList.contains("preview-loading"));
const control = () => $("#preview-versions");
const title = () => control().querySelector(".preview-version-title")?.textContent;
const switcher = () => $("#preview-files");
const versionsOf = (cmd: string) => calls().argsOf(cmd).map((args) => args.version);

async function preview(name: string): Promise<void> {
  await click(hit(name));
  await key(" ");
  await until(shown);
  await settle(10);
}

beforeAll(async () => {
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  await boot();
});

afterEach(async () => {
  await closeOverlays();
  if (!$("#preview-layer").hidden) await key("Escape");
  calls().reset();
  viewer.showModel.mockReset();
  viewer.showModel.mockImplementation(fakeView);
});

viewer.showModel.mockImplementation(fakeView);

test("the model is asked for, and shown, before the version history is read", async () => {
  await goTo(SHAPES);
  calls().hold("arbesk_versions");
  await preview("Bracket.gltf");
  // The walk is still waiting: the scene, its part and the files already came.
  expect(calls().count("arbesk_versions")).toBe(1);
  expect(versionsOf("arbesk_scene")).toEqual([null]);
  expect(versionsOf("arbesk_part")).toEqual(["bracket-v2"]);
  expect($("#preview-body canvas.preview-model")).toBeTruthy();
  await until(() => !switcher().hidden);
  expect(versionsOf("arbesk_representations")).toEqual(["bracket-v2"]);
  expect(control().hidden).toBe(true);
  calls().release("arbesk_versions");
  await until(() => !control().hidden);
  expect(title()).toBe("Version 2 of 2");
  expect(calls().count("arbesk_scene")).toBe(1);
});

test("the version control comes even when one of the save's files is shown by then", async () => {
  await goTo(SHAPES);
  calls().hold("arbesk_versions");
  await preview("Bracket.gltf");
  await until(() => !switcher().hidden);
  await click(switcher().querySelector<HTMLButtonElement>("button")!);
  const row = [...document.querySelectorAll(".popover .menu-item")].find((el) => el.querySelector(".label")?.textContent === "Bracket.blend");
  await click(row as HTMLElement);
  await until(() => !!$("#preview-body .preview-file-page"));
  calls().release("arbesk_versions");
  await until(() => !control().hidden);
  expect(title()).toBe("Version 2 of 2");
  expect($("#preview-body .preview-file-page")).toBeTruthy();
});

test("when the newest save can't be read, the preview says so and has no switcher", async () => {
  await goTo(SHAPES);
  calls().failNext("arbesk_scene", "Arbesk can't be reached. Check your connection.");
  await preview("Bracket.gltf");
  await settle(50);
  expect($("#preview-body").textContent).toContain("No preview available.");
  expect(switcher().hidden).toBe(true);
  expect(calls().count("arbesk_representations")).toBe(0);
  expect(calls().count("arbesk_part")).toBe(0);
});

// From here on, each test's save stays in the mock for the tests after it:
// Bracket has 3 saves after this one, 4 after the next, 5 after the last.
test("a save that lands before the walk ends shows in the control; the preview stays on its save", async () => {
  await goTo(SHAPES);
  calls().hold("arbesk_versions");
  await preview("Bracket.gltf");
  await until(() => !switcher().hidden);
  const drawn = viewer.showModel.mock.calls.length;
  // Another device saves the asset again before the walk reads it.
  mockArbesk().saveAgain("1", "bracket");
  calls().release("arbesk_versions");
  await until(() => !control().hidden);
  await settle(20);
  expect(title()).toBe("Version 2 of 3");
  expect(control().querySelector<HTMLButtonElement>("[aria-label='Newer Version']")!.disabled).toBe(false);
  // The scene, its parts and the switcher's files all from bracket-v2; nothing swapped.
  expect(versionsOf("arbesk_scene")).toEqual([null]);
  expect(versionsOf("arbesk_part")).toEqual(["bracket-v2"]);
  expect(versionsOf("arbesk_representations")).toEqual(["bracket-v2"]);
  expect(viewer.showModel.mock.calls.length).toBe(drawn);
  // The newer save is one step away.
  await click(control().querySelector<HTMLButtonElement>("[aria-label='Newer Version']")!);
  await until(shown);
  await until(() => !switcher().hidden);
  expect(title()).toBe("Version 3 of 3");
  expect(versionsOf("arbesk_scene").at(-1)).toBe("bracket-v3");
  expect(versionsOf("arbesk_representations").at(-1)).toBe("bracket-v3");
});

test("a save that lands between the scene and the files' read keeps the files on the scene's save", async () => {
  await goTo(SHAPES);
  calls().hold("arbesk_representations");
  await preview("Bracket.gltf");
  await until(() => calls().count("arbesk_representations") === 1);
  expect(versionsOf("arbesk_scene")).toEqual([null]);
  // The scene answered bracket-v3; another device saves before the files are read.
  mockArbesk().saveAgain("1", "bracket");
  calls().release("arbesk_representations");
  await until(() => !switcher().hidden);
  expect(versionsOf("arbesk_representations")).toEqual(["bracket-v3"]);
  expect(versionsOf("arbesk_part")).toEqual(["bracket-v3"]);
});

test("a save that lands between the walk and the scene: the saves are read again, the control is drawn", async () => {
  await goTo(SHAPES);
  calls().hold("arbesk_scene");
  await click(hit("Bracket.gltf"));
  await key(" ");
  await until(() => calls().count("arbesk_versions") === 1);
  await settle(20);
  // The walk read bracket-v4 as the newest; the scene, still waiting, reads after this save.
  mockArbesk().saveAgain("1", "bracket");
  calls().release("arbesk_scene");
  await until(shown);
  await until(() => !control().hidden);
  expect(title()).toBe("Version 5 of 5");
  expect(calls().count("arbesk_versions")).toBe(2);
  expect(versionsOf("arbesk_part")).toEqual(["bracket-v5"]);
});

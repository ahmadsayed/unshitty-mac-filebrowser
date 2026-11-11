// Space on an Arbesk asset with other files (spec F14): the header's
// switcher lists the model and each file of the version on screen; a file
// the viewer draws previews as a model, any other shows its file page with
// Open With…, which fetches the file from that version; the version menu
// says what each save changed. The real app against the mock, with only
// the WebGL drawing (showModel) replaced, as jsdom has none.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { ARBESK_MOCK_FILES, MOCK_ARBESK_APPS, invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, closeOverlays, dialogHeading, goTo, hit, key, settle } from "./app";

const viewer = vi.hoisted(() => ({ showModel: vi.fn() }));

vi.mock("../src/model-viewer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/model-viewer")>();
  return { ...actual, showModel: viewer.showModel };
});

type MockCalls = { count(cmd: string): number; argsOf(cmd: string): Record<string, unknown>[]; failNext(cmd: string, error: string): void; reset(): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
type MockApps = { set(o: { list?: { name: string; id: string }[] }): void; opened(): { path: string; app: string }[]; reset(): void };
const mockApps = () => (globalThis as unknown as { __mockApps: MockApps }).__mockApps;
type MockArbesk = { changeFile(token: string, asset: string, file: string): void };
const mockArbesk = () => (globalThis as unknown as { __mockArbesk: MockArbesk }).__mockArbesk;

const SHAPES = "arbesk://0xabababababababababababababababababababab/1~Shapes";
const GCODE = "Bracket_0.2mm_PETG_MK4.gcode";
const GCODE_CID = "bafyrepbracketxgcodeoriginalprusamk4petg";

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
const switcher = () => $("#preview-files");
const switcherButton = () => switcher().querySelector<HTMLButtonElement>("button")!;
const menuRows = () =>
  $$(".popover .preview-files-menu .menu-item").map((el) => ({
    title: el.querySelector(".label")!.textContent,
    subtitle: el.querySelector(".menu-item-subtitle")?.textContent ?? "",
    shown: !!el.querySelector(".radio-dot"),
  }));

async function preview(name: string): Promise<void> {
  await click(hit(name));
  await key(" ");
  await until(shown);
  await settle(10);
}

/** Pick `title` in the switcher's menu and wait for what it shows. */
async function pick(title: string): Promise<void> {
  await click(switcherButton());
  const row = $$(".popover .menu-item").find((el) => el.querySelector(".label")?.textContent === title);
  if (!row) throw new Error(`no file “${title}” in ${JSON.stringify(menuRows())}`);
  await click(row);
  await until(shown);
  await settle(20);
}

beforeAll(async () => {
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  await boot();
});

afterEach(async () => {
  await closeOverlays();
  if (!$("#preview-layer").hidden) await key("Escape");
  viewer.showModel.mockReset();
  viewer.showModel.mockImplementation(fakeView);
  mockApps().reset();
  calls().reset();
});

viewer.showModel.mockImplementation(fakeView);

test("an asset with other files gets the switcher, showing the model; one without has none", async () => {
  await goTo(SHAPES);
  await preview("Bracket.gltf");
  await until(() => !switcher().hidden);
  expect(switcherButton().textContent).toBe("Model");
  expect(switcherButton().getAttribute("aria-haspopup")).toBe("menu");
  await click(switcherButton());
  expect(menuRows()).toEqual([
    { title: "Bracket.gltf", subtitle: "Model", shown: true },
    { title: "Bracket.blend", subtitle: "Blender file · 96.2 kB · Source", shown: false },
    { title: "Bracket.stl", subtitle: "STL · 5.7 kB · Mesh", shown: false },
    {
      title: GCODE,
      subtitle: "G-code · 1.2 MB · For the printer · PrusaSlicer 2.9.2 · Original Prusa MK4, PETG, 0.2 mm layers, 0.4 mm nozzle · 2 h 3 min · 38.5 g",
      shown: false,
    },
    { title: "Bracket.3mf", subtitle: "3MF · 2.2 kB · Mesh", shown: false },
  ]);
  await closeOverlays();
  await key("Escape");
  // Box.3mf has no other files: no switcher, and no list asked for.
  const before = calls().count("arbesk_representations");
  await preview("Box.3mf");
  await settle(30);
  expect(switcher().hidden).toBe(true);
  expect(calls().count("arbesk_representations")).toBe(before);
});

test("the first view and the current version picked again load the scene by the save its files come from", async () => {
  await goTo(SHAPES);
  await preview("Bracket.gltf");
  await until(() => !switcher().hidden);
  // The first view: the scene of the newest save, asked for at once, says
  // which save that is; its parts and files are read by that CID, never
  // the asset's path, so a save made meanwhile can't mix with the files.
  expect(calls().argsOf("arbesk_scene")).toEqual([expect.objectContaining({ version: null })]);
  expect(calls().argsOf("arbesk_part").map((args) => args.version)).toEqual(["bracket-v2"]);
  expect(calls().argsOf("arbesk_representations").at(-1)).toMatchObject({ version: "bracket-v2" });
  // Older, then Newer again: the current save, by its CID too.
  await click($("#preview-versions").querySelector<HTMLButtonElement>("[aria-label='Older Version']")!);
  await until(shown);
  await until(() => !switcher().hidden);
  expect(calls().argsOf("arbesk_scene").at(-1)).toMatchObject({ version: "bracket-v1" });
  await click($("#preview-versions").querySelector<HTMLButtonElement>("[aria-label='Newer Version']")!);
  await until(shown);
  await until(() => !switcher().hidden);
  expect(calls().argsOf("arbesk_scene").at(-1)).toMatchObject({ version: "bracket-v2" });
  expect(calls().argsOf("arbesk_representations").at(-1)).toMatchObject({ version: "bracket-v2" });
});

test("an STL is fetched from the version on screen and previewed as a model", async () => {
  await goTo(SHAPES);
  await preview("Bracket.gltf");
  await until(() => !switcher().hidden);
  const before = viewer.showModel.mock.calls.length;
  const fetches = calls().count("arbesk_file");
  await pick("Bracket.stl");
  expect(calls().count("arbesk_file")).toBe(fetches + 1);
  // From the shown save, under an operation id no other window's fetch shares.
  const fetched = calls().argsOf("arbesk_file").at(-1)!;
  expect(fetched).toMatchObject({ version: "bracket-v2", id: "stl" });
  expect(fetched.op).toMatch(/^preview-file-[0-9a-z]+-[0-9a-z]+-\d+$/);
  expect(viewer.showModel.mock.calls.length).toBe(before + 1);
  expect($("#preview-body canvas.preview-model")).toBeTruthy();
  expect(switcherButton().textContent).toBe("STL");
  await click(switcherButton());
  expect(menuRows().find((r) => r.shown)?.title).toBe("Bracket.stl");
  await closeOverlays();
  // Back to the model: the scene again, of the shown save by its CID even
  // when it is the current one, so the model and the files listed beside
  // it come from one save.
  await pick("Bracket.gltf");
  expect(switcherButton().textContent).toBe("Model");
  expect(viewer.showModel.mock.calls.length).toBe(before + 2);
  expect(calls().argsOf("arbesk_scene").at(-1)).toMatchObject({ version: "bracket-v2" });
  // Each fetch its own id.
  await pick("Bracket.stl");
  const ops = calls().argsOf("arbesk_file").map((args) => args.op);
  expect(new Set(ops).size).toBe(ops.length);
});

test("a .blend shows the file page; a G-code's says it is out of date, and so does the switcher", async () => {
  await goTo(SHAPES);
  await preview("Bracket.gltf");
  await until(() => !switcher().hidden);
  const fetches = calls().count("arbesk_file");
  await pick("Bracket.blend");
  const page = $("#preview-body .preview-file-page");
  expect(page.querySelector(".preview-file-name")?.textContent).toBe("Bracket.blend");
  expect(page.querySelector(".preview-file-subtitle")?.textContent).toBe("Blender file · 96.2 kB · Source");
  expect(page.querySelector(".out-of-date-tag")).toBeNull();
  expect([...page.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["Open With…"]);
  // Nothing is fetched for a page.
  expect(calls().count("arbesk_file")).toBe(fetches);
  expect(switcherButton().textContent).toBe("Blender file");
  expect(switcher().querySelector(".preview-files-dot")).toBeNull();
  await pick(GCODE);
  const tag = $("#preview-body .preview-file-page .out-of-date-tag");
  expect(tag.textContent).toBe("Out of date");
  expect(tag.title).toBe("Made from an earlier version of the model");
  expect(switcherButton().textContent).toBe("G-code");
  expect(switcher().querySelector(".preview-files-dot")).toBeTruthy();
  expect(switcherButton().getAttribute("aria-label")).toBe("Shown File: G-code, out of date");
});

test("the switcher's menu is reached and used with the keyboard", async () => {
  await goTo(SHAPES);
  await preview("Bracket.gltf");
  await until(() => !switcher().hidden);
  switcherButton().focus();
  // A button: Enter or Space opens its menu (the browser's click), the arrows move in it.
  await click(switcherButton());
  const menu = $(".popover .menu");
  await key("ArrowDown", { target: menu });
  expect((document.activeElement as HTMLElement).querySelector(".label")?.textContent).toBe("Bracket.gltf");
  await key("ArrowDown", { target: document.activeElement! });
  expect((document.activeElement as HTMLElement).querySelector(".label")?.textContent).toBe("Bracket.blend");
  await click(document.activeElement as HTMLElement);
  await until(shown);
  await settle(20);
  expect($("#preview-body .preview-file-page")).toBeTruthy();
  // Drawn again, the switcher keeps the keyboard; Escape on it closes the preview.
  expect(document.activeElement).toBe(switcherButton());
  await key("Escape", { target: switcherButton() });
  expect($("#preview-layer").hidden).toBe(true);
});

test("stepping to another version shows its model again, with that version's files", async () => {
  await goTo(SHAPES);
  await preview("Bracket.gltf");
  await until(() => !switcher().hidden);
  await pick("Bracket.blend");
  await click($("#preview-versions").querySelector<HTMLButtonElement>("[aria-label='Older Version']")!);
  await until(shown);
  await until(() => !switcher().hidden);
  expect($("#preview-body canvas.preview-model")).toBeTruthy();
  expect(switcherButton().textContent).toBe("Model");
  await click(switcherButton());
  // Version 1 had the source and the G-code, made from its own model.
  expect(menuRows().map((r) => r.title)).toEqual(["Bracket.gltf", "Bracket.blend", GCODE]);
  await closeOverlays();
  await pick(GCODE);
  expect($("#preview-body .out-of-date-tag")).toBeNull();
});

test("the preview's Open still opens the asset in the Studio", async () => {
  const log = vi.spyOn(console, "log");
  await goTo(SHAPES);
  await preview("Bracket.gltf");
  await until(() => !switcher().hidden);
  await pick("Bracket.blend");
  const openButton = [...$("#preview-actions").querySelectorAll("button")].find((b) => b.textContent === "Open")!;
  await click(openButton);
  await settle(20);
  expect(log.mock.calls.some(([what, url]) => what === "mock open_url" && String(url).includes("promptscad.com"))).toBe(true);
  log.mockRestore();
});

test("the version menu says what each save changed in the files", async () => {
  await goTo(SHAPES);
  await preview("Bracket.gltf");
  await until(() => !$("#preview-versions").hidden);
  await click($("#preview-versions").querySelector<HTMLButtonElement>(".preview-version-label")!);
  const rows = $$(".popover .preview-version-menu .menu-item").map((el) => [
    el.querySelector(".label")!.textContent,
    el.querySelector(".menu-item-subtitle")?.textContent ?? "",
  ]);
  expect(rows).toEqual([
    ["Version 2 — Current", "Added STL · Added 3MF"],
    ["Version 1", "Added Blender file · Added G-code (Original Prusa MK4, PETG)"],
  ]);
});

test("a save whose predecessor can't be read says no change", async () => {
  // The backend's walk (service.rs, tested there) sends no `changes` then;
  // the menu shows the row alone.
  const { versionMenu, fromArbeskSaves } = await import("../src/preview-versions");
  const items = versionMenu(fromArbeskSaves([{ id: "a", timestamp: null }, { id: "b", timestamp: null, changes: ["Added STL"] }]), 1, () => {});
  expect(items.map((item) => ("subtitle" in item ? item.subtitle : undefined))).toEqual(["Added STL", undefined]);
});

test("after a file's id was replaced, the earlier version's file is the one opened", async () => {
  // The newest save's G-code is now another file; version 1 keeps its own.
  mockArbesk().changeFile("1", "bracket", "gcode-original-prusa-mk4-petg");
  mockApps().set({ list: [{ name: "TextEdit", id: "/Applications/TextEdit.app" }, ...MOCK_ARBESK_APPS] });
  await goTo(SHAPES);
  await preview("Bracket.gltf");
  await until(() => !switcher().hidden);
  await click($("#preview-versions").querySelector<HTMLButtonElement>("[aria-label='Older Version']")!);
  await until(shown);
  await until(() => !switcher().hidden);
  await pick(GCODE);
  const exports = calls().count("arbesk_export");
  await click([...$$("#preview-body .preview-file-page button")].find((b) => b.textContent === "Open With…")!);
  await settle(20);
  expect(dialogHeading()).toBe(`Open “${GCODE}” With`);
  await click($$(".app-choice").find((el) => el.textContent === "Text Editor")!);
  await settle(40);
  expect(mockApps().opened()).toEqual([{ path: `${ARBESK_MOCK_FILES}/${GCODE_CID}/${GCODE}`, app: "/Applications/Text Editor.app" }]);
  // Never the current save's path: the version's own fetch.
  expect(calls().count("arbesk_export")).toBe(exports);
  // The newest version's G-code is the replaced one.
  await click($("#preview-versions").querySelector<HTMLButtonElement>("[aria-label='Newer Version']")!);
  await until(shown);
  await until(() => !switcher().hidden);
  await pick(GCODE);
  await click([...$$("#preview-body .preview-file-page button")].find((b) => b.textContent === "Open With…")!);
  await settle(20);
  await click($$(".app-choice").find((el) => el.textContent === "Text Editor")!);
  await settle(40);
  expect(mockApps().opened().at(-1)).toEqual({ path: `${ARBESK_MOCK_FILES}/${GCODE_CID}x2/${GCODE}`, app: "/Applications/Text Editor.app" });
});

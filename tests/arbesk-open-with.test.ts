// Open With an Arbesk asset (spec F8): which file the chosen application
// gets (the model when it declares none, the one it opens, or a choice when
// it opens several), fetched into the app cache as an operation and handed
// over without landing in Recent; and the Files… page's Open With….
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { ARBESK_MOCK_BUILDS, ARBESK_MOCK_FILES, MOCK_ARBESK_APPS, invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, chooseMenu, click, closeOverlays, dialogHeading, installFakeCanvas, key, menuLabels, open, respond, rightClick, settle, toasts } from "./app";

type MockApps = { set(o: { list?: { name: string; id: string }[]; failOpen?: string }): void; opened(): { path: string; app: string }[]; reset(): void };
const mockApps = () => (globalThis as unknown as { __mockApps: MockApps }).__mockApps;
type MockCalls = { count(cmd: string): number; failNext(cmd: string, error: string): void; reset(): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
type MockOps = { hold(): void; held(): number; step(bytes: number): void; release(): void; reset(): void };
const ops = () => (globalThis as unknown as { __mockOps: MockOps }).__mockOps;

const APPS = [{ name: "TextEdit", id: "/Applications/TextEdit.app" }, ...MOCK_ARBESK_APPS];
const GCODE = "Bracket_0.2mm_PETG_MK4.gcode";
const GCODE_FILE = `${ARBESK_MOCK_FILES}/bafyrepbracketxgcodeoriginalprusamk4petg/${GCODE}`;

const row = (name: string) => $$("#view .item").find((el) => el.querySelector(".name")?.textContent === name)!;
const appRow = (name: string) => $$(".app-choice").find((el) => el.textContent === name)!;
const choiceRows = () =>
  $$(".files-dialog .choice-row").map((el) => ({
    title: el.querySelector(".action-row-title")!.textContent,
    checked: el.querySelector<HTMLInputElement>("input")!.checked,
    stale: !!el.querySelector(".out-of-date-tag"),
  }));

beforeAll(async () => {
  installFakeCanvas();
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  await boot();
  await key("1", { mod: true });
  await click($$(".sidebar-row").find((el) => el.textContent?.trim() === "Network")!);
  await settle(20);
  await open("Arbesk");
  await settle(20);
  await open("Shapes");
  await settle(20);
});

afterEach(async () => {
  ops().release();
  ops().reset();
  calls().reset();
  await settle(10);
  await closeOverlays();
  mockApps().reset();
});

/** Open With… on `asset`, then pick `app` by clicking its row. */
async function openWith(asset: string, app: string): Promise<void> {
  mockApps().set({ list: APPS });
  await click(row(asset));
  await rightClick(row(asset));
  await chooseMenu("Open With…");
  await settle(20);
  expect(dialogHeading()).toBe(`Open “${asset}” With`);
  await click(appRow(app));
  await settle(30);
}

test("an asset's menu offers Open With…; an app that declares none of its files gets the model", async () => {
  await click(row("Bracket.gltf"));
  await rightClick(row("Bracket.gltf"));
  expect(menuLabels()).toEqual(["Open", "Open With…", "Copy", "Properties"]);
  await closeOverlays();
  const recentBefore = calls().count("add_recent");
  await openWith("Bracket.gltf", "TextEdit");
  // The chooser closed first; the model was built and handed over, not added to Recent.
  expect(dialogHeading()).toBeUndefined();
  expect(mockApps().opened()).toEqual([{ path: `${ARBESK_MOCK_BUILDS}/bracket/Bracket.gltf`, app: "/Applications/TextEdit.app" }]);
  expect(calls().count("add_recent")).toBe(recentBefore);
});

test("an app that opens one of the files gets it, without a question", async () => {
  await openWith("Bracket.gltf", "Text Editor");
  expect(dialogHeading()).toBeUndefined();
  expect(mockApps().opened()).toEqual([{ path: GCODE_FILE, app: "/Applications/Text Editor.app" }]);
});

test("an app that opens several asks which file, the first one not out of date chosen; Cancel opens nothing", async () => {
  await openWith("Bracket.gltf", "PrusaSlicer");
  expect(dialogHeading()).toBe("Open Which File With PrusaSlicer?");
  expect($$(".open-with-dialog")).toHaveLength(0);
  expect(choiceRows()).toEqual([
    { title: "Bracket.stl", checked: true, stale: false },
    { title: GCODE, checked: false, stale: true },
    { title: "Bracket.3mf", checked: false, stale: false },
  ]);
  expect($$(".files-dialog .dialog-responses button").map((b) => b.textContent)).toEqual(["Cancel", "Open"]);
  await key("Escape");
  await settle(20);
  expect(dialogHeading()).toBeUndefined();
  expect(mockApps().opened()).toEqual([]);

  // Again, choosing the G-code: Open hands it over.
  await openWith("Bracket.gltf", "PrusaSlicer");
  await click($$(".files-dialog .choice-row")[1]);
  await respond("Open");
  await settle(30);
  expect(mockApps().opened()).toEqual([{ path: GCODE_FILE, app: "/Applications/PrusaSlicer.app" }]);
});

test("started with Open, the chooser closes first; the model counts as never out of date; Enter opens", async () => {
  mockApps().set({ list: APPS });
  await click(row("Bracket.gltf"));
  await rightClick(row("Bracket.gltf"));
  await chooseMenu("Open With…");
  await settle(20);
  await click(appRow("Blender"));
  await settle(30);
  expect(dialogHeading()).toBe("Open Which File With Blender?");
  expect(choiceRows().map((r) => [r.title, r.checked])).toEqual([
    ["Bracket.gltf", true], ["Bracket.blend", false], ["Bracket.stl", false],
  ]);
  await key("Enter");
  await settle(30);
  expect(mockApps().opened()).toEqual([{ path: `${ARBESK_MOCK_BUILDS}/bracket/Bracket.gltf`, app: "/Applications/Blender.app" }]);

  // The chooser's own Open button does the same.
  await click(row("Box.gltf"));
  await rightClick(row("Box.gltf"));
  await chooseMenu("Open With…");
  await settle(20);
  mockApps().set({ list: APPS });
  await key("ArrowDown", { target: $(".app-search") });
  await respond("Open");
  await settle(30);
  expect($$(".open-with-dialog")).toHaveLength(0);
  // PrusaSlicer opens Box.gltf's one STL, not its glTF model.
  expect(mockApps().opened()[1]).toEqual({ path: `${ARBESK_MOCK_FILES}/bafyrepboxgxstl/Box.stl`, app: "/Applications/PrusaSlicer.app" });
});

test("the fetch is an operation with progress and Cancel; a cancel says nothing", async () => {
  ops().hold();
  await openWith("Bracket.gltf", "Text Editor");
  await vi.waitFor(() => expect(ops().held()).toBe(1), { timeout: 2000 });
  await click($("#btn-operations"));
  await settle(5);
  expect($(".operation-row .operation-title").textContent).toBe(`Opening “${GCODE}”`);
  await click($(".operation-row .operation-cancel"));
  await settle(30);
  expect(mockApps().opened()).toEqual([]);
  expect(toasts().some((t) => t.startsWith("Could not open"))).toBe(false);
});

test("a failed fetch or launch is a toast naming the file", async () => {
  calls().failNext("arbesk_export", "Failed: The gateway did not answer.");
  await openWith("Bracket.gltf", "Text Editor");
  expect(toasts()).toContain(`Could not open “${GCODE}”: The gateway did not answer.`);
  mockApps().set({ list: APPS, failOpen: "The application can't be opened." });
  await click(row("Box.gltf"));
  await rightClick(row("Box.gltf"));
  await chooseMenu("Open With…");
  await settle(20);
  await click(appRow("TextEdit"));
  await settle(30);
  expect(toasts()).toContain("Could not open “Box.gltf”: The application can't be opened.");
});

test("the Files… page opens a chosen file with any app, without asking which", async () => {
  await click(row("Bracket.gltf"));
  await key("i", { mod: true });
  await settle(10);
  await respond("Files…");
  await settle(10);
  mockApps().set({ list: APPS });
  const blend = $$(".files-dialog .files-row").find((el) => el.querySelector(".action-row-title")?.textContent === "Bracket.blend")!;
  await click(blend.querySelector(".files-row-menu")!);
  expect(menuLabels()).toEqual(["Open With…", "Copy"]);
  // Escape closes the row's menu, not the page beneath it.
  await key("Escape");
  await settle(5);
  expect($$(".popover")).toHaveLength(0);
  expect(dialogHeading()).toBe("Files of “Bracket.gltf”");
  await click(blend.querySelector(".files-row-menu")!);
  await chooseMenu("Open With…");
  await settle(20);
  expect(dialogHeading()).toBe("Open “Bracket.blend” With");
  await click(appRow("PrusaSlicer"));
  await settle(30);
  expect(mockApps().opened()).toEqual([{ path: `${ARBESK_MOCK_FILES}/bafyrepbracketxsource/Bracket.blend`, app: "/Applications/PrusaSlicer.app" }]);
  // The model's row hands over the model.
  await click($$(".files-dialog .files-row-menu")[0]);
  await chooseMenu("Open With…");
  await settle(20);
  expect(dialogHeading()).toBe("Open “Bracket.gltf” With");
  await click(appRow("PrusaSlicer"));
  await settle(30);
  expect(mockApps().opened()[1]).toEqual({ path: `${ARBESK_MOCK_BUILDS}/bracket/Bracket.gltf`, app: "/Applications/PrusaSlicer.app" });
});

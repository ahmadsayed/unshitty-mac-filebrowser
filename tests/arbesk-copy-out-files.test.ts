// Copying an Arbesk asset's other files out (spec F9): the dialog asks
// which files when an asset has some (at paste time, so a paste after
// leaving the collection asks too, as does a drag out of it); the
// targets' names (one plain file name, repeats made unique, clashes with
// the folder still asked); a pinned file that changed since Copy refused.
import { beforeAll, expect, test } from "vitest";
import { ARBESK_MOCK_ADDRESS, invoke } from "../dev/mock-tauri/core";
import { $$, boot, chooseMenu, click, closeOverlays, dialogHeading, goTo, installFakeCanvas, key, mouse, names, respond, settle, toasts } from "./app";

type MockArbesk = {
  addAsset(token: string, id: string, name: string, files: Array<{ id: string; format: string; name: string; bytes: number }>): void;
  changeFile(token: string, asset: string, file: string): void;
};
const mockArbesk = () => (globalThis as unknown as { __mockArbesk: MockArbesk }).__mockArbesk;

const HOME = "/Users/me";
const SHAPES = `arbesk://${ARBESK_MOCK_ADDRESS}/1~Shapes`;
const GCODE = "Bracket_0.2mm_PETG_MK4.gcode";

const row = (name: string) => $$("#view .item").find((el) => el.querySelector(".name")?.textContent === name)!;
const checkRows = () =>
  $$(".files-dialog .choice-row").map((el) => {
    const box = el.querySelector<HTMLInputElement>("input")!;
    return [el.querySelector(".action-row-title")!.textContent, box.indeterminate ? "mixed" : box.checked];
  });
const box = (title: string) =>
  $$(".files-dialog .choice-row").find((el) => el.querySelector(".action-row-title")?.textContent === title)!.querySelector<HTMLInputElement>("input")!;
const copyButton = () => $$<HTMLButtonElement>(".files-dialog .dialog-responses button").find((b) => b.textContent === "Copy")!;

beforeAll(async () => {
  installFakeCanvas();
  mockArbesk().addAsset("1", "odd", "Odd", [
    { id: "gcode", format: "gcode", name: "parts/Odd.gcode", bytes: 300 },
    { id: "stl", format: "stl", name: "Box.stl", bytes: 400 },
  ]);
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  for (const dir of ["Out1", "Out2", "Out3", "Out4", "Out5", "Out6"]) await invoke("make_dir", { path: `${HOME}/${dir}` });
  await boot();
  await key("1", { mod: true });
});

/** Select `picks` in Shapes and copy them. */
async function copyFromShapes(picks: string[]): Promise<void> {
  await goTo(SHAPES);
  await settle(20);
  await click(row(picks[0]));
  for (const name of picks.slice(1)) await click(row(name), { metaKey: true });
  await key("c", { mod: true });
}

async function pasteIn(dir: string): Promise<void> {
  await goTo(`${HOME}/${dir}`);
  await key("v", { mod: true });
  await settle(30);
}

test("one asset: a check row per file, the model checked, All Files mixed; Copy needs a check", async () => {
  await copyFromShapes(["Bracket.gltf"]);
  // Pasted after leaving the collection: it still asks.
  await pasteIn("Out1");
  expect(dialogHeading()).toBe("Copy Which Files of “Bracket.gltf”?");
  expect(checkRows()).toEqual([
    ["All Files", "mixed"], ["Bracket.gltf", true], ["Bracket.blend", false], ["Bracket.stl", false], [GCODE, false], ["Bracket.3mf", false],
  ]);
  expect($$(".files-dialog .choice-row .action-row-subtitle").map((el) => el.textContent)[3]).toBe("STL · 5.7 kB · Mesh");
  await click(box("Bracket.gltf"));
  expect(checkRows()[0]).toEqual(["All Files", false]);
  expect(copyButton().disabled).toBe(true);
  await click(box("All Files"));
  expect(checkRows().every(([, checked]) => checked === true)).toBe(true);
  expect(copyButton().disabled).toBe(false);
  await click(box("All Files"));
  expect(copyButton().disabled).toBe(true);
  for (const title of ["Bracket.gltf", "Bracket.stl", GCODE]) await click(box(title));
  expect(checkRows()[0]).toEqual(["All Files", "mixed"]);
  await respond("Copy");
  await settle(40);
  expect(names().sort()).toEqual(["Bracket.gltf", "Bracket.stl", GCODE].sort());
  expect(toasts().some((t) => t.startsWith("Could not"))).toBe(false);
});

test("Cancel copies nothing; an asset without other files copies with no question", async () => {
  await copyFromShapes(["Bracket.gltf"]);
  await pasteIn("Out2");
  await key("Escape");
  await settle(30);
  expect(names()).toEqual([]);
  await copyFromShapes(["Box.3mf"]);
  await pasteIn("Out2");
  expect(dialogHeading()).toBeUndefined();
  expect(names()).toEqual(["Box.3mf"]);
});

test("several assets: Models and a row per format, applied to each; repeated names made unique", async () => {
  await copyFromShapes(["Box.gltf", "Odd.gltf"]);
  await pasteIn("Out3");
  expect(dialogHeading()).toBe("Copy Which Files?");
  expect(checkRows()).toEqual([["All Files", "mixed"], ["Models", true], ["STL (2 assets)", false], ["G-code (1 asset)", false]]);
  await click(box("STL (2 assets)"));
  await click(box("G-code (1 asset)"));
  await respond("Copy");
  await settle(40);
  // Each asset's Box.stl lands, the second as Keep Both names it; a name
  // with a slash lands as one plain file in the folder.
  expect(names().sort()).toEqual(["Box (copy).stl", "Box.gltf", "Box.stl", "Odd.gltf", "parts_Odd.gcode"].sort());
});

test("files already in the folder still ask Replace, Skip or Keep Both", async () => {
  await copyFromShapes(["Box.gltf"]);
  await pasteIn("Out4");
  await click(box("All Files"));
  await respond("Copy");
  await settle(40);
  expect(names().sort()).toEqual(["Box.gltf", "Box.stl"]);
  await key("v", { mod: true });
  await settle(30);
  await click(box("All Files"));
  await respond("Copy");
  await settle(30);
  expect(dialogHeading()).toBe("Replace file “Box.gltf”?");
  await respond("Skip");
  await settle(30);
  expect(dialogHeading()).toBe("Replace file “Box.stl”?");
  await respond("Keep Both");
  await settle(40);
  expect(names().sort()).toEqual(["Box (copy).stl", "Box.gltf", "Box.stl"]);
});

test("a drag out of the collection asks too", async () => {
  const pointAt = document.elementFromPoint;
  const desktop = $$(".sidebar-row").find((r) => r.dataset.location === `${HOME}/Desktop`)!;
  let target: Element | null = null;
  document.elementFromPoint = () => target;
  try {
    await goTo(SHAPES);
    await settle(20);
    await click(row("Box.gltf"));
    await mouse(row("Box.gltf"), "mousedown", { clientX: 50, clientY: 60 });
    target = desktop;
    for (const at of [300, 301]) {
      window.dispatchEvent(new MouseEvent("mousemove", { clientX: at, clientY: at }));
      await settle(10);
    }
    window.dispatchEvent(new MouseEvent("mouseup", { clientX: 301, clientY: 301 }));
    await settle(30);
    expect(dialogHeading()).toBe("Copy Which Files of “Box.gltf”?");
    await click(box("Box.stl"));
    await respond("Copy");
    await settle(40);
    expect(await invoke("path_exists", { path: `${HOME}/Desktop/Box.stl` })).toBe(true);
    expect(await invoke("path_exists", { path: `${HOME}/Desktop/Box.gltf` })).toBe(true);
  } finally {
    document.elementFromPoint = pointAt;
  }
});

test("the Files… page's Copy takes one file; once it changed in Arbesk, the paste is refused", async () => {
  await goTo(SHAPES);
  await settle(20);
  await click(row("Bracket.gltf"));
  await key("i", { mod: true });
  await settle(10);
  await respond("Files…");
  await settle(10);
  const stl = $$(".files-dialog .files-row").find((el) => el.querySelector(".action-row-title")?.textContent === "Bracket.stl")!;
  await click(stl.querySelector(".files-row-menu")!);
  await chooseMenu("Copy");
  await closeOverlays();
  await pasteIn("Out6");
  // A file, not an asset: no question.
  expect(dialogHeading()).toBeUndefined();
  expect(names()).toEqual(["Bracket.stl"]);

  mockArbesk().changeFile("1", "bracket", "stl");
  await key("v", { mod: true });
  await settle(30);
  expect(dialogHeading()).toBe("Replace file “Bracket.stl”?");
  await respond("Keep Both");
  await settle(40);
  expect(toasts()).toContain("Could not copy “Bracket.stl”: “Bracket.stl” changed in Arbesk since it was copied.");
  expect(names()).toEqual(["Bracket.stl"]);
});

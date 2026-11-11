// Properties of an Arbesk asset (spec F7): from its menu and Ctrl+I, its
// Version row and type line and none of a local file's facts; its Files…
// page, read only; collections still have none.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, closeOverlays, dialogHeading, installFakeCanvas, key, menuLabels, open, respond, rightClick, settle, toasts } from "./app";

const row = (name: string) => $$("#view .item").find((el) => el.querySelector(".name")?.textContent === name)!;
const propertyRows = () =>
  Object.fromEntries($$(".properties-dialog .prop-row").map((el) => [el.querySelector(".prop-label")!.textContent, el.querySelector(".prop-value")!.textContent]));
const dialogButtons = () => $$(".dialog-layer:last-child .dialog-responses button").map((b) => b.textContent);

beforeAll(async () => {
  installFakeCanvas();
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  await boot();
  await key("1", { mod: true });
});

async function openArbesk(collection?: string): Promise<void> {
  await click($$(".sidebar-row").find((el) => el.textContent?.trim() === "Network")!);
  await settle(20);
  await open("Arbesk");
  await settle(20);
  if (collection) {
    await open(collection);
    await settle(20);
  }
}

test("an asset's menu ends with Properties, which shows its version and no local file facts", async () => {
  await openArbesk("Shapes");
  await click(row("Bracket.gltf"));
  await rightClick(row("Bracket.gltf"));
  expect(menuLabels()).toEqual(["Open", "Open With…", "Copy", "Paste Into Asset", "Properties"]);
  expect($$(".popover .menu-separator").length).toBe(1);
  await click($$(".popover .menu-item").find((el) => el.textContent?.trim() === "Properties")!);
  await settle(10);
  expect(dialogHeading()).toBe("Properties");
  expect($(".properties-name").textContent).toBe("Bracket.gltf");
  expect($(".properties-type").textContent).toBe("Arbesk asset (glTF)");
  const rows = propertyRows();
  // The collection, by name; its address stays in the tooltip.
  expect(rows["Parent folder"]).toBe("Shapes");
  expect(rows.Version).toBe("2");
  expect(Object.keys(rows)).toEqual(["Parent folder", "Modified", "Version"]);
  for (const absent of ["Size", "Accessed", "Created", "Permissions", "Group", "Others", "Owner", "Link target"]) {
    expect(rows[absent], absent).toBeUndefined();
  }
  expect(dialogButtons()).toEqual(["Files…", "Close"]);
  await closeOverlays();
});

test("Ctrl+I opens it too; a 3MF asset is named as one and one without other files has no Files…", async () => {
  await click(row("Box.3mf"));
  await key("i", { mod: true });
  await settle(10);
  expect(dialogHeading()).toBe("Properties");
  expect($(".properties-type").textContent).toBe("Arbesk asset (3MF)");
  expect(propertyRows().Version).toBe("1");
  expect(dialogButtons()).toEqual(["Close"]);
  await closeOverlays();
});

test("Files… lists the model first, then each file with its subtitle and out-of-date tag", async () => {
  await click(row("Bracket.gltf"));
  await key("i", { mod: true });
  await settle(10);
  await respond("Files…");
  await settle(10);
  expect(dialogHeading()).toBe("Files of “Bracket.gltf”");
  const rows = $$(".files-dialog .files-row").map((el) => ({
    title: el.querySelector(".action-row-title")!.textContent,
    subtitle: el.querySelector(".action-row-subtitle")!.textContent,
    tag: el.querySelector<HTMLElement>(".out-of-date-tag")?.title ?? null,
  }));
  expect(rows).toEqual([
    { title: "Bracket.gltf", subtitle: "Model", tag: null },
    { title: "Bracket.blend", subtitle: "Blender file · 96.2 kB · Source", tag: null },
    { title: "Bracket.stl", subtitle: "STL · 5.7 kB · Mesh", tag: null },
    {
      title: "Bracket_0.2mm_PETG_MK4.gcode",
      subtitle: "G-code · 1.2 MB · For the printer · PrusaSlicer 2.9.2 · Original Prusa MK4, PETG, 0.2 mm layers, 0.4 mm nozzle · 2 h 3 min · 38.5 g",
      tag: "Made from an earlier version of the model",
    },
    { title: "Bracket.3mf", subtitle: "3MF · 2.2 kB · Mesh", tag: null },
  ]);
  expect($(".files-dialog .out-of-date-tag").textContent).toBe("Out of date");
  // Each row has a menu (Open With…, Copy, spec F8 and F9), then Close.
  expect($$(".files-dialog .files-row-menu").map((b) => b.getAttribute("aria-label"))).toEqual(
    rows.map((r) => `Menu for “${r.title}”`),
  );
  expect($$(".files-dialog .dialog-responses button").map((b) => b.textContent)).toEqual(["Close"]);
  await click($(".files-dialog .dialog-responses button"));
  await settle(10);
  // Properties is still open beneath it.
  expect(dialogHeading()).toBe("Properties");
  await closeOverlays();
});

type MockCalls = { hold(cmd: string): void; release(cmd: string): void; count(cmd: string): number; failNext(cmd: string, error: string): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
const button = (label: string) => $$<HTMLButtonElement>(".properties-dialog .dialog-responses button").find((b) => b.textContent === label)!;

test("Files… waits on Arbesk once: a second press is ignored and one page opens", async () => {
  await click(row("Bracket.gltf"));
  await key("i", { mod: true });
  await settle(10);
  calls().hold("arbesk_representations");
  const before = calls().count("arbesk_representations");
  await click(button("Files…"));
  await settle(10);
  expect(button("Files…").disabled).toBe(true);
  expect(button("Close").disabled).toBe(false);
  await click(button("Files…"));
  await settle(10);
  calls().release("arbesk_representations");
  await settle(20);
  expect(calls().count("arbesk_representations")).toBe(before + 1);
  expect($$(".files-dialog")).toHaveLength(1);
  expect(button("Files…").disabled).toBe(false);
  await closeOverlays();
});

test("Properties closed while Files… waits opens no page; a failure is a toast", async () => {
  await click(row("Bracket.gltf"));
  await key("i", { mod: true });
  await settle(10);
  calls().hold("arbesk_representations");
  await click(button("Files…"));
  await settle(10);
  await click(button("Close"));
  await settle(10);
  expect(dialogHeading()).toBeUndefined();
  calls().release("arbesk_representations");
  await settle(20);
  expect($$(".files-dialog")).toHaveLength(0);
  await key("i", { mod: true });
  await settle(10);
  calls().failNext("arbesk_representations", "Failed to read the asset: The gateway did not answer.");
  await respond("Files…");
  await settle(10);
  expect($$(".files-dialog")).toHaveLength(0);
  expect(toasts()).toContain("Could not show the files: The gateway did not answer.");
  expect(dialogHeading()).toBe("Properties");
  await closeOverlays();
});

test("Properties closed while Files… waits shows no failure either", async () => {
  await click(row("Bracket.gltf"));
  await key("i", { mod: true });
  await settle(10);
  for (const t of document.querySelectorAll(".toast")) t.remove();
  calls().hold("arbesk_representations");
  calls().failNext("arbesk_representations", "Failed to read the asset: The gateway did not answer.");
  await click(button("Files…"));
  await settle(10);
  await click(button("Close"));
  await settle(10);
  calls().release("arbesk_representations");
  await settle(20);
  expect(dialogHeading()).toBeUndefined();
  expect(toasts()).not.toContain("Could not show the files: The gateway did not answer.");
});

test("the Files page opens after the wait and gives focus back to Files… when closed", async () => {
  await click(row("Bracket.gltf"));
  await key("i", { mod: true });
  await settle(10);
  calls().hold("arbesk_representations");
  button("Files…").focus();
  await click(button("Files…"));
  await settle(10);
  // WebKit drops focus from a disabled control to the page; jsdom keeps it
  // there and will not blur a disabled control, so the test drops it.
  const files = button("Files…");
  expect(files.disabled).toBe(true);
  files.disabled = false;
  files.blur();
  files.disabled = true;
  expect(document.activeElement).toBe(document.body);
  calls().release("arbesk_representations");
  await settle(20);
  expect(dialogHeading()).toBe("Files of “Bracket.gltf”");
  await key("Escape");
  await settle(10);
  expect(dialogHeading()).toBe("Properties");
  expect(document.activeElement).toBe(button("Files…"));
  await closeOverlays();
});

test("the Version row is the manifest's version, not a count of saves, and none for a fractional one", async () => {
  await click(row("Big.gltf"));
  await key("i", { mod: true });
  await settle(10);
  expect(propertyRows().Version).toBeUndefined();
  await closeOverlays();
  await openArbesk("Untitled Collection");
  await click(row("Table.gltf"));
  await key("i", { mod: true });
  await settle(10);
  // One save, numbered 5 by its manifest.
  expect(propertyRows().Version).toBe("5");
  await closeOverlays();
});

test("a collection and the account still have no Properties", async () => {
  await openArbesk();
  await click(row("Shapes"));
  await key("i", { mod: true });
  await settle(10);
  expect(dialogHeading()).toBeUndefined();
  await rightClick(row("Shapes"));
  expect(menuLabels()).not.toContain("Properties");
  await closeOverlays();
  expect(toasts()).not.toContain("Properties aren't available for Arbesk yet.");
});

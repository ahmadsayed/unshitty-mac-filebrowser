// An Arbesk asset that has other files (spec F6): the "has other files"
// emblem with its tooltip, the out-of-date dot, on a thumbnail too, in the
// grid and list views, drawn from the listing alone so a virtualized view
// draws the same markup whenever a row comes back.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, goTo, installFakeCanvas, key, open, settle } from "./app";

const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));
const arbeskMock = () => (globalThis as unknown as { __mockArbesk: { addManyAssets(count: number): string } }).__mockArbesk;
const row = (name: string) => $$("#view .item").find((el) => el.querySelector(".name")?.textContent === name)!;
const filesEmblem = (name: string) => row(name).querySelector<HTMLElement>(".icon .emblem-files");

beforeAll(async () => {
  installFakeCanvas(); // the mock draws the thumbnails Arbesk hands over
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  await boot();
});

async function openShapes(): Promise<void> {
  await click($$(".sidebar-row").find((el) => el.textContent?.trim() === "Network")!);
  await settle(20);
  await open("Arbesk");
  await settle(20);
  await open("Shapes");
  await settle(20);
}

for (const [view, shortcut] of [["list", "1"], ["grid", "2"]] as const) {
  test(`the ${view} view: the emblem, its tooltip and the out-of-date dot`, async () => {
    await openShapes();
    await key(shortcut, { mod: true });
    await settle(20);
    // Box has one fresh STL, on the thumbnail it was saved with.
    const box = filesEmblem("Box.gltf")!;
    expect(row("Box.gltf").querySelector(".icon img.thumb")).not.toBeNull();
    expect(box.title).toBe("Has 1 other file");
    expect(box.classList.contains("stale")).toBe(false);
    expect(box.querySelector("use")?.getAttribute("href")).toBe("#sym-emblem-files");
    // Bracket: four files the app reads (one it can't is left out), its G-code out of date.
    const bracket = filesEmblem("Bracket.gltf")!;
    expect(bracket.title).toBe("Has 4 other files, 1 out of date");
    expect(bracket.classList.contains("stale")).toBe(true);
    // The emblem comes last, after the picture.
    expect(row("Bracket.gltf").querySelector(".icon")!.lastElementChild).toBe(bracket);
    // Assets without other files carry none.
    expect(filesEmblem("Box.3mf")).toBeNull();
    expect(filesEmblem("Big.gltf")).toBeNull();
  });
}

test("the markup of a row is the same when a virtualized list draws it again", async () => {
  const many = arbeskMock().addManyAssets(2000);
  await goTo(many);
  await key("1", { mod: true });
  await settle(20);
  const scroller = $("#view-scroll");
  const icons = () => new Map($$("#view .item").map((el) => [el.querySelector(".name")!.textContent!, el.querySelector(".icon")!.innerHTML]));
  const before = icons();
  expect(before.size).toBeGreaterThan(0);
  expect(before.size).toBeLessThan(200);
  expect(before.get("Part 0000.gltf")).not.toContain("emblem-files");
  expect(before.get("Part 0001.gltf")).toContain('title="Has 1 other file"');
  expect(before.get("Part 0002.gltf")).toContain("emblem emblem-files stale");

  for (const top of [30_000, 0]) {
    scroller.scrollTop = top;
    scroller.dispatchEvent(new Event("scroll"));
    await pause(50);
    await settle(10);
    if (top > 0) expect(icons().has("Part 0000.gltf")).toBe(false);
  }
  const after = icons();
  for (const [name, markup] of before) expect(after.get(name), name).toBe(markup);
});

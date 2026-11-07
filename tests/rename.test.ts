// Batch rename: find/replace, prefix/suffix across a multi-selection.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, chooseMenu, click, dialogHeading, goTo, item, key, menuLabels, mouse, names, respond, settle, type, hit } from "./app";

const HOME = "/Users/me";
const DOCS = `${HOME}/Documents`;

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

test("several selected files get a prefix and a suffix before the extension", async () => {
  await goTo(DOCS);
  await click(hit("Notes.md"));
  await click(hit("Roadmap.key"), { metaKey: true });
  await rightClick(item("Roadmap.key"));
  expect(menuLabels()).toContain("Rename…");
  await chooseMenu("Rename…");
  await settle(10);
  expect(dialogHeading()).toBe("Rename 2 Items");

  await type($<HTMLInputElement>("#batch-prefix"), "copy-");
  await type($<HTMLInputElement>("#batch-suffix"), " (old)");
  expect($(".batch-preview").textContent).toContain("copy-Notes (old).md");
  expect($(".batch-preview").textContent).toContain("copy-Roadmap (old).key");
  await respond("Rename");
  await settle(30);

  const found = names();
  expect(found).toContain("copy-Notes (old).md");
  expect(found).toContain("copy-Roadmap (old).key");
  expect(found).not.toContain("Notes.md");
  expect(found).not.toContain("Roadmap.key");
});

test("a clashing new name is refused before anything is renamed", async () => {
  await goTo(DOCS);
  await click(hit("Budget 2026.xlsx"));
  await click(hit("copy-Notes (old).md"), { metaKey: true });
  await rightClick(item("copy-Notes (old).md"));
  await chooseMenu("Rename…");
  await settle(10);

  await type($<HTMLInputElement>("#batch-find"), "Budget 2026.xlsx");
  await type($<HTMLInputElement>("#batch-replace"), "copy-Notes (old).md");
  expect($(".dialog-layer").textContent).toContain("overwrite another selected item");
  const rename = $$(".dialog-layer .btn").find((button) => button.textContent === "Rename")!;
  expect(rename.hasAttribute("disabled")).toBe(true);
  await respond("Cancel");
  await settle(10);
  expect(names()).toContain("Budget 2026.xlsx");
});

test("starred items keep their star, and so does a starred item inside a renamed folder", async () => {
  // Quarterly report.pdf and Projects start starred; star Budget 2026.xlsx and Projects/website too.
  await goTo(`${DOCS}/Projects`);
  await mouse(item("website").querySelector(".star")!, "mousedown");
  await settle(10);
  await goTo(DOCS);
  await mouse(item("Budget 2026.xlsx").querySelector(".star")!, "mousedown");
  await settle(10);

  await click(hit("Quarterly report.pdf"));
  await click(hit("Budget 2026.xlsx"), { metaKey: true });
  await click(hit("Projects"), { metaKey: true });
  await rightClick(item("Projects"));
  await chooseMenu("Rename…");
  await settle(10);
  expect(dialogHeading()).toBe("Rename 3 Items");
  await type($<HTMLInputElement>("#batch-prefix"), "old-");
  await respond("Rename");
  await settle(30);

  for (const name of ["old-Quarterly report.pdf", "old-Budget 2026.xlsx", "old-Projects"]) {
    expect(item(name).querySelector(".star")!.classList.contains("starred")).toBe(true);
  }
  const saved = await invoke<string[]>("load_starred");
  expect(saved).toEqual(expect.arrayContaining([
    `${DOCS}/old-Quarterly report.pdf`,
    `${DOCS}/old-Budget 2026.xlsx`,
    `${DOCS}/old-Projects`,
    `${DOCS}/old-Projects/website`,
  ]));
  for (const path of [`${DOCS}/Quarterly report.pdf`, `${DOCS}/Budget 2026.xlsx`, `${DOCS}/Projects`, `${DOCS}/Projects/website`]) {
    expect(saved).not.toContain(path);
  }

  await goTo(`${DOCS}/old-Projects`);
  expect(item("website").querySelector(".star")!.classList.contains("starred")).toBe(true);
});

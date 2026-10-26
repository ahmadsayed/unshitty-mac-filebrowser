// Duplicate, Create Link and Compress, all beside the selection.
import { beforeAll, expect, test } from "vitest";
import { $, $$, boot, chooseMenu, click, dialogHeading, goTo, item, key, mouse, names, respond, settle, hit } from "./app";

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

test("Duplicate makes a copy and Create Link makes a symlink", async () => {
  await goTo(DOCS);
  await rightClick(item("Notes.md"));
  await chooseMenu("Duplicate");
  await settle(30);
  expect(names()).toContain("Notes (copy).md");

  await rightClick(item("Notes.md"));
  await chooseMenu("Create Link");
  await settle(30);
  expect(names()).toContain("Link to Notes.md");
});

test("Compress creates a zip and can switch to tar.gz", async () => {
  await goTo(DOCS);
  await click(hit("Notes.md"));
  await click(hit("Budget 2026.xlsx"), { metaKey: true });
  await rightClick(item("Budget 2026.xlsx"));
  await chooseMenu("Compress…");
  await settle(10);
  expect(dialogHeading()).toBe("Compress 2 Items");
  expect($<HTMLInputElement>("#compress-name").value).toBe("Archive.zip");

  const format = $<HTMLSelectElement>("#compress-format");
  format.value = "tar.gz";
  format.dispatchEvent(new Event("change", { bubbles: true }));
  expect($<HTMLInputElement>("#compress-name").value).toBe("Archive.tar.gz");
  await respond("Compress");
  await settle(30);
  expect(names()).toContain("Archive.tar.gz");
});

test("New Document creates an empty file and copies a template", async () => {
  await goTo(DOCS);
  await rightClick($("#view"));
  await chooseMenu("New Document…");
  await settle(20);
  expect(dialogHeading()).toBe("New Document");
  const empty = $$(".app-choice").find((button) => button.textContent === "Empty Text File")!;
  await click(empty);
  await settle(30);
  expect(names()).toContain("Untitled.txt");

  await rightClick($("#view"));
  await chooseMenu("New Document…");
  await settle(20);
  const letter = $$(".app-choice").find((button) => button.textContent === "Letter.odt")!;
  await click(letter);
  await settle(30);
  expect(names()).toContain("Letter.odt");
});

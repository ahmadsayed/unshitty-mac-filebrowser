// Archive extraction: into a new folder and here, with no overwrite.
import { beforeAll, expect, test } from "vitest";
import { boot, chooseMenu, goTo, item, key, menuLabels, mouse, names, open, settle, title } from "./app";

const HOME = "/Users/me";
const DOWNLOADS = `${HOME}/Downloads`;

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

test("an archive offers Extract Here and Extract to a new folder", async () => {
  await goTo(DOWNLOADS);
  await rightClick(item("photos-export.zip"));
  const labels = menuLabels();
  expect(labels).toContain("Extract Here");
  expect(labels).toContain("Extract to “photos-export”");

  await chooseMenu("Extract to “photos-export”");
  await settle(30);
  expect(names()).toContain("photos-export");
  await open("photos-export");
  expect(title()).toBe("photos-export");
  expect(names()).toEqual(["extracted.txt"]);
});

test("Extract Here puts the files beside the archive", async () => {
  await goTo(DOWNLOADS);
  await rightClick(item("photos-export.zip"));
  await chooseMenu("Extract Here");
  await settle(30);
  expect(names()).toContain("extracted.txt");
});

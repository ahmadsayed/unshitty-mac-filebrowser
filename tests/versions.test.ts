// Keep Versions UI: the New Folder checkbox, the Versions… dialog (every diff
// state, Restore Copy, Replace), Deleted Files…, and the Properties rows.
import { beforeAll, expect, test } from "vitest";
import { restoreName, restoreStamp } from "../src/versions";
import { $, $$, boot, chooseMenu, click, closeOverlays, dialogHeading, goTo, item, key, mouse, names, respond, settle, toasts, type } from "./app";

const VERSIONED = "/Users/me/Documents/Versioned";

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

async function openVersions(name: string): Promise<void> {
  await goTo(VERSIONED);
  await rightClick(item(name));
  await chooseMenu("Versions…");
  await settle(10);
}

const rows = () => $$(".versions-dialog .version-row");
const diffText = () => $(".versions-dialog .versions-diff").textContent ?? "";
const button = (label: string) =>
  $$(".dialog-layer .dialog-responses button").find((b) => b.textContent?.trim() === label) as HTMLButtonElement;

test("restore names are local, sortable and have no colon", () => {
  const stamp = restoreStamp(0, new Date(2026, 9, 24, 14, 5));
  expect(stamp).toBe("2026-10-24 14-05");
  const at = new Date(2026, 0, 2, 3, 4).getTime() / 1000;
  expect(restoreName("/a/notes.txt", at)).toBe("notes (2026-01-02 03-04).txt");
  expect(restoreName("/a/Makefile", at)).toBe("Makefile (2026-01-02 03-04)");
  expect(restoreName("/a/.hidden", at)).toBe(".hidden (2026-01-02 03-04)");
});

test("the Versions… dialog lists versions and shows each diff state", async () => {
  await openVersions("notes.txt");
  expect(dialogHeading()).toBe("Versions of “notes.txt”");
  expect(rows().length).toBe(4);
  expect(rows()[0].textContent).toContain("Current");
  expect(rows()[3].textContent).toContain("Deleted");
  // The newest is selected first and is the current file.
  expect(diffText()).toBe("This version is the same as the current file.");
  expect(button("Restore Copy").disabled).toBe(true);
  // An older version: a line diff, and both actions are available.
  await click(rows()[1]);
  await settle(10);
  expect($$(".versions-diff .diff-line.removed").map((l) => l.textContent)).toEqual(["-old line"]);
  expect($$(".versions-diff .diff-line.added").map((l) => l.textContent)).toEqual(["+new line"]);
  expect(button("Restore Copy").disabled).toBe(false);
  expect(button("Replace…").disabled).toBe(false);
  await click(rows()[2]);
  await settle(10);
  expect(diffText()).toContain("first draft");
  // The deletion record: nothing to restore.
  await click(rows()[3]);
  await settle(10);
  expect(diffText()).toBe("The file was deleted at this point.");
  expect(button("Restore Copy").disabled).toBe(true);
  expect(button("Replace…").disabled).toBe(true);
  await closeOverlays();
});

test("fast clicks never leave a stale diff", async () => {
  await openVersions("notes.txt");
  rows()[1].click();
  rows()[3].click(); // answered after the first request
  await settle(15);
  expect(diffText()).toBe("The file was deleted at this point.");
  await closeOverlays();
});

test("binary and large files say why there is no diff", async () => {
  await openVersions("image.png");
  expect(diffText()).toBe("No preview for this type of file.");
  await closeOverlays();
  await openVersions("large.txt");
  await click(rows()[1]);
  await settle(10);
  expect(diffText()).toBe("This file is too large to compare.");
  await closeOverlays();
});

test("Restore Copy adds a dated copy and refuses to overwrite it", async () => {
  await openVersions("notes.txt");
  await click(rows()[1]);
  await settle(10);
  await respond("Restore Copy");
  await settle(10);
  expect(toasts().join(" ")).toMatch(/Restored as “notes \(\d{4}-\d\d-\d\d \d\d-\d\d\)\.txt”/);
  expect(names().some((n) => /^notes \(.*\)\.txt$/.test(n))).toBe(true);
  // Same version again: the name is taken, so the mock refuses.
  await openVersions("notes.txt");
  await click(rows()[1]);
  await settle(10);
  await respond("Restore Copy");
  await settle(10);
  expect(toasts().join(" ")).toMatch(/Could not restore/);
});

test("Replace… asks first, with Cancel as the default", async () => {
  await openVersions("notes.txt");
  await click(rows()[2]);
  await settle(10);
  await respond("Replace…");
  await settle(10);
  expect(dialogHeading()).toBe("Replace “notes.txt”?");
  expect($(".dialog-layer").textContent).toMatch(/moved to the Trash/);
  await respond("Cancel");
  expect(dialogHeading()).toBeUndefined();
  await openVersions("notes.txt");
  await click(rows()[2]);
  await settle(10);
  await respond("Replace…");
  await settle(10);
  await respond("Replace");
  await settle(10);
  expect(toasts().join(" ")).toMatch(/Replaced “notes.txt”/);
});

test("Deleted Files… restores, then reports a clash", async () => {
  await goTo(VERSIONED);
  await rightClick($("#view"));
  await chooseMenu("Deleted Files…");
  expect(dialogHeading()).toBe("Deleted Files");
  expect($(".deleted-dialog .deleted-name").textContent).toBe("draft.md");
  await click($(".deleted-dialog .deleted-row button"));
  await settle(10);
  expect(toasts().join(" ")).toMatch(/Restored “draft.md”/);
  expect($$(".deleted-dialog .deleted-row").length).toBe(0);
  await closeOverlays();
  await rightClick($("#view"));
  await chooseMenu("Deleted Files…");
  await click($(".deleted-dialog .deleted-row button"));
  await settle(10);
  expect(toasts().join(" ")).toMatch(/Could not restore/);
  await closeOverlays();
});

test("Properties shows a file's version count and the root's history", async () => {
  await goTo(VERSIONED);
  await rightClick(item("notes.txt"));
  await chooseMenu("Properties");
  await settle(15);
  const rowText = () => $$(".dialog-layer .prop-row").map((r) => r.textContent);
  expect(rowText()).toContain("Versions4");
  await closeOverlays();
  await goTo("/Users/me/Documents");
  await rightClick(item("Versioned"));
  await chooseMenu("Properties");
  await settle(15);
  expect(rowText().join(" ")).toMatch(/Version History12\.4 MB, 214 versions of 37 files/);
  await click($$(".dialog-layer button").find((b) => b.textContent === "Clear History…")!);
  expect(dialogHeading()).toBe("Clear Version History?");
  await respond("Clear History");
  expect(toasts().join(" ")).toMatch(/Version history cleared/);
  expect(document.querySelector(".dialog-layer")).toBeNull();
});

test("Stop Keeping Versions… turns the folder back into a plain one", async () => {
  await goTo("/Users/me/Documents");
  await rightClick(item("Versioned"));
  await chooseMenu("Properties");
  await settle(15);
  await click($$(".dialog-layer button").find((b) => b.textContent === "Stop Keeping Versions…")!);
  expect(dialogHeading()).toBe("Stop Keeping Versions?");
  await respond("Stop Keeping Versions");
  expect(toasts().join(" ")).toMatch(/Stopped keeping versions/);
  await goTo(VERSIONED);
  await rightClick(item("notes.txt"));
  expect($$(".popover .menu-item").map((b) => b.textContent)).not.toContain("Versions…");
  await closeOverlays();
});

test("New Folder can keep versions, and isn't offered inside a versioned folder", async () => {
  await goTo("/Users/me");
  await key("N", { mod: true, shift: true });
  const keep = $<HTMLInputElement>("#new-folder-versions");
  expect(keep.disabled).toBe(false);
  keep.checked = true;
  await type($<HTMLInputElement>("#new-folder-name"), "Tracked");
  await respond("Create");
  expect(names()).toContain("Tracked");
  await goTo("/Users/me/Tracked");
  await key("N", { mod: true, shift: true });
  expect($<HTMLInputElement>("#new-folder-versions").disabled).toBe(true);
  expect($(".dialog-layer").textContent).toContain("Already inside a folder that keeps versions");
  await closeOverlays();
});

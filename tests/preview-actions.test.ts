// Actions in the Space preview (#48): Open, Rename…, Move to Trash and Open
// Item Location, offered where the item's place allows them, also for
// search results that live in subfolders.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import type { LocationKind } from "../src/locations";
import { previewActions, previewPlace } from "../src/preview-actions";
import { $, $$, boot, click, dialogHeading, goTo, hit, key, names, respond, selectedNames, settle, toasts, type } from "./app";

const ROOT = "/Users/me/PreviewActions";
const pause = (ms = 260) => new Promise((resolve) => setTimeout(resolve, ms));

const buttons = () => $$("#preview-actions button").map((b) => b.textContent);
const press = async (label: string) => {
  const button = $$("#preview-actions button").find((b) => b.textContent === label);
  if (!button) throw new Error(`no preview button “${label}” in ${buttons().join(", ")}`);
  await click(button);
  await settle(10);
};
const exists = (path: string) => invoke<boolean>("path_exists", { path });

beforeAll(async () => {
  for (const dir of ["", "/Work"]) await invoke("make_dir", { path: ROOT + dir });
  for (const file of ["/plan.txt", "/plan-b.txt", "/old.txt", "/Work/plan-work.txt", "/Work/taken.txt"]) {
    await invoke("create_file", { path: ROOT + file });
  }
  await boot();
  await key("1", { mod: true }); // list view
});

async function preview(name: string): Promise<void> {
  await click(hit(name));
  await key(" ");
  await settle(20);
  expect($("#preview-layer").hidden).toBe(false);
}

async function search(query: string): Promise<void> {
  await goTo(ROOT);
  await key("f", { mod: true });
  await type($<HTMLInputElement>("#search-entry"), query);
  await pause();
  await settle(10);
}

test("which actions a place offers", () => {
  const offers = (kind: LocationKind, item: LocationKind, elsewhere = false) =>
    previewActions(previewPlace({ kind, items: [item], elsewhere }));
  expect(offers("folder", "folder")).toEqual(["rename", "trash", "open"]);
  expect(offers("folder", "folder", true)).toEqual(["location", "rename", "trash", "open"]);
  // Read-only shares and SFTP: nothing that changes the item.
  expect(offers("share", "share")).toEqual(["open"]);
  expect(offers("sftp", "sftp", true)).toEqual(["location", "open"]);
  // Recent and Starred: no Rename, as in their menus.
  expect(offers("recent", "folder", true)).toEqual(["location", "trash", "open"]);
  expect(offers("starred", "folder", true)).toEqual(["location", "trash", "open"]);
  // In the Trash: Restore and Delete Permanently stay in the menu.
  expect(offers("trash", "in-trash", true)).toEqual(["open"]);
  expect(offers("in-trash", "in-trash")).toEqual(["open"]);
  expect(offers("recent", "in-trash", true)).toEqual(["open"]);
  // Kept: a Drive item's preview has neither Rename… nor Move to Trash, which its menu offers.
  expect(offers("drive", "drive")).toEqual(["open"]);
  // Kept: the Trash folder itself, shown with hidden files, gets Rename…, which F2 refuses.
  expect(offers("folder", "trash")).toEqual(["rename", "trash", "open"]);
  expect(offers("starred", "trash", true)).toEqual(["location", "trash", "open"]);
});

test("a file in this folder offers Rename…, Move to Trash and Open", async () => {
  await goTo(ROOT);
  await preview("plan.txt");
  expect(buttons()).toEqual(["Rename…", "Move to Trash", "Open"]);
  await key("Escape");
  await settle(10);
});

test("Rename… checks for clashes, keeps the preview on the renamed file", async () => {
  await goTo(ROOT);
  await preview("plan.txt");
  await press("Rename…");
  expect(dialogHeading()).toBe("Rename File");
  const input = $<HTMLInputElement>(".dialog input");
  expect(input.value).toBe("plan.txt");

  await type(input, "plan-b.txt");
  expect($(".dialog .field-message").textContent).toBe("A file with that name already exists.");
  expect($<HTMLButtonElement>(".dialog-responses button.suggested").disabled).toBe(true);

  await type(input, "plan-final.txt");
  await respond("Rename");
  await settle(20);
  expect($$(".dialog-layer")).toHaveLength(0);
  expect(await exists(`${ROOT}/plan-final.txt`)).toBe(true);
  expect(await exists(`${ROOT}/plan.txt`)).toBe(false);
  expect($("#preview-layer").hidden).toBe(false);
  expect($("#preview-name").textContent).toBe("plan-final.txt");
  expect(selectedNames()).toEqual(["plan-final.txt"]);
  await key("Escape");
  await settle(10);
});

test("Move to Trash closes the preview and can be undone", async () => {
  await goTo(ROOT);
  await preview("old.txt");
  await press("Move to Trash");
  await settle(20);
  expect($("#preview-layer").hidden).toBe(true);
  expect(names()).not.toContain("old.txt");
  expect(toasts().some((t) => t.includes("“old.txt” moved to trash"))).toBe(true);
  expect(document.activeElement).toBe($("#view"));

  const undo = $$(".toast button").find((b) => b.textContent === "Undo")!;
  await click(undo);
  await settle(20);
  expect(names()).toContain("old.txt");
});

test("Open closes the preview and opens the file", async () => {
  await goTo(ROOT);
  await preview("plan-b.txt");
  await press("Open");
  expect($("#preview-layer").hidden).toBe(true);
  expect(toasts().some((t) => t.startsWith("Could not open"))).toBe(false);
});

test("a search result in a subfolder offers Open Item Location, which selects it there", async () => {
  await search("plan-work");
  await preview("plan-work.txt");
  expect(buttons()).toEqual(["Open Item Location", "Rename…", "Move to Trash", "Open"]);
  await press("Open Item Location");
  await settle(20);
  expect($("#preview-layer").hidden).toBe(true);
  expect($("#searchbar").hidden).toBe(true);
  expect(names()).toEqual(["plan-work.txt", "taken.txt"]);
  expect(selectedNames()).toEqual(["plan-work.txt"]);
});

test("Rename… on a search result in a subfolder keeps it in its own folder", async () => {
  await search("plan-work");
  await preview("plan-work.txt");
  await press("Rename…");
  const input = $<HTMLInputElement>(".dialog input");
  // The clash is in the item's own folder, not the folder searched from.
  await type(input, "taken.txt");
  await respond("Rename");
  await settle(20);
  expect($(".dialog .field-message").textContent).toBe("A file with that name already exists.");
  expect(await exists(`${ROOT}/Work/plan-work.txt`)).toBe(true);

  await type(input, "plan-renamed.txt");
  await respond("Rename");
  await settle(20);
  expect(await exists(`${ROOT}/Work/plan-renamed.txt`)).toBe(true);
  expect(await exists(`${ROOT}/plan-renamed.txt`)).toBe(false);
  await key("Escape");
  await settle(10);
});

test("F2 on a search result in a subfolder renames it in its own folder", async () => {
  await search("plan-renamed");
  await click(hit("plan-renamed.txt"));
  await key("F2");
  await settle(10);
  const input = $<HTMLInputElement>(".rename-popover input");
  await type(input, "plan-f2.txt");
  await key("Enter", { target: input });
  await settle(20);
  expect(await exists(`${ROOT}/Work/plan-f2.txt`)).toBe(true);
  expect(await exists(`${ROOT}/plan-f2.txt`)).toBe(false);
});

test("Escape on a preview button closes the preview and returns to the view", async () => {
  await key("Escape", { target: $("#search-entry") });
  await goTo(ROOT);
  await preview("plan-b.txt");
  const button = $$("#preview-actions button")[0];
  button.focus();
  await key("Escape", { target: button });
  await settle(10);
  expect($("#preview-layer").hidden).toBe(true);
  expect(document.activeElement).toBe($("#view"));
});

test("a Recent item offers Open Item Location, Move to Trash and Open", async () => {
  await click($$(".sidebar-row").find((r) => r.textContent?.trim() === "Recent")!);
  await settle(20);
  await preview("Notes.md");
  expect(buttons()).toEqual(["Open Item Location", "Move to Trash", "Open"]);
  await key("Escape");
  await settle(10);
});

test("items in the Trash only offer Open", async () => {
  await goTo("/Users/me/.Trash");
  await settle(10);
  await preview("old.txt");
  expect(buttons()).toEqual(["Open"]);
  await key("Escape");
});

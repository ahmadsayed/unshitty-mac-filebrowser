// File dialogs while their work runs: Compress, Batch Rename, New Folder and
// Rename from the preview. Shared helpers live in tests/support/dialog-in-flight.ts.
import { beforeAll, beforeEach, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, chooseMenu, click, dialogHeading, hit, item, key, names, respond, rightClick, settle, toasts, type } from "./app";
import { button, calls, cleanDialogs, folder, message, setupDialogs } from "./support/dialog-in-flight";

beforeAll(setupDialogs);
beforeEach(cleanDialogs);

// ----- Compress -----

async function openCompress(name: string): Promise<void> {
  await click(hit(name));
  await rightClick(item(name));
  await chooseMenu("Compress…");
}

test("Compress: Escape while it runs closes the dialog, and the archive still appears", async () => {
  await folder("compress-escape");
  await openCompress("a.txt");
  calls().hold("compress_archive");
  await key("Enter", { target: $("#compress-name") });
  expect(dialogHeading()).toBe("Compress 1 Item");
  expect(button("Cancel").disabled).toBe(false);
  await key("Escape", { target: $("#compress-name") });
  expect(dialogHeading()).toBeUndefined();
  calls().release("compress_archive");
  await settle(30);
  expect(names()).toContain("a.txt.zip");
  expect(toasts().join(" ")).toContain("Created “a.txt.zip”");
});

test("Compress: a failure stays in the dialog, cleaned, with everything enabled", async () => {
  await folder("compress-fail");
  await openCompress("a.txt");
  calls().failNext("compress_archive", "Failed to write the archive: No space left on device (os error 28)");
  await respond("Compress");
  expect(dialogHeading()).toBe("Compress 1 Item");
  expect(message()).toBe("No space left on device");
  expect(button("Compress").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  expect($<HTMLInputElement>("#compress-name").disabled).toBe(false);
  expect($<HTMLSelectElement>("#compress-format").disabled).toBe(false);
});

// ----- Batch Rename -----

async function openBatchRename(): Promise<void> {
  await click(hit("a.txt"));
  await click(hit("b.txt"), { metaKey: true });
  await rightClick(item("b.txt"));
  await chooseMenu("Rename…");
  await type($<HTMLInputElement>("#batch-prefix"), "x-");
}

test("Batch Rename: Escape while it runs closes the dialog, and the items are still renamed", async () => {
  await folder("batch-escape");
  await openBatchRename();
  calls().hold("rename_many");
  await key("Enter", { target: $("#batch-prefix") });
  expect(dialogHeading()).toBe("Rename 2 Items");
  await key("Escape", { target: $("#batch-prefix") });
  expect(dialogHeading()).toBeUndefined();
  calls().release("rename_many");
  await settle(30);
  expect(names()).toEqual(expect.arrayContaining(["x-a.txt", "x-b.txt"]));
});

test("Batch Rename: a failure stays in the dialog, cleaned, with everything enabled", async () => {
  await folder("batch-fail");
  await openBatchRename();
  calls().failNext("rename_many", "Failed to rename: Permission denied (os error 13)");
  await respond("Rename");
  expect(dialogHeading()).toBe("Rename 2 Items");
  expect(message()).toBe("Permission denied");
  expect(button("Rename").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  expect($<HTMLInputElement>("#batch-prefix").disabled).toBe(false);
  expect(names()).toEqual(expect.arrayContaining(["a.txt", "b.txt"]));
});

test("Batch Rename: after a failure the view shows the folder as it is now", async () => {
  // A rollback the backend refused leaves an item renamed (#192); here the
  // folder changes as the batch fails, and the view must not keep old names.
  const dir = await folder("batch-partial");
  await openBatchRename();
  await invoke("rename_entry", { from: `${dir}/a.txt`, to: `${dir}/kept.txt` });
  calls().failNext("rename_many", "Failed to rename: Permission denied (os error 13)");
  await respond("Rename");
  await settle(30);
  expect(dialogHeading()).toBe("Rename 2 Items");
  expect(names()).toEqual(expect.arrayContaining(["kept.txt", "b.txt"]));
  expect(names()).not.toContain("a.txt");
});

// ----- New Folder -----

async function openNewFolder(name: string): Promise<HTMLInputElement> {
  await key("N", { mod: true, shift: true });
  const input = $<HTMLInputElement>("#new-folder-name");
  await type(input, name);
  return input;
}

test("New Folder: Escape while it runs closes the dialog, and the folder still appears", async () => {
  await folder("new-escape", []);
  const input = await openNewFolder("Made");
  calls().hold("make_dir");
  await key("Enter", { target: input });
  expect(dialogHeading()).toBe("New Folder");
  await key("Escape", { target: input });
  expect(dialogHeading()).toBeUndefined();
  calls().release("make_dir");
  await settle(30);
  expect(names()).toEqual(["Made"]);
});

test("New Folder: a failure stays in the dialog, cleaned, with everything enabled", async () => {
  await folder("new-fail", []);
  const input = await openNewFolder("Made");
  calls().failNext("make_dir", "Failed to create directory: Permission denied (os error 13)");
  await respond("Create");
  expect(dialogHeading()).toBe("New Folder");
  expect(message()).toBe("Permission denied");
  expect($(".dialog-layer .field-message").classList.contains("error")).toBe(true);
  expect(button("Create").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  expect(input.disabled).toBe(false);
  expect(toasts().join(" ")).not.toContain("Permission denied");
});

// ----- Rename from the preview -----

async function openPreviewRename(name: string): Promise<HTMLInputElement> {
  await click(hit(name));
  await key(" ");
  await settle(20);
  const rename = $$("#preview-actions button").find((b) => b.textContent === "Rename…")!;
  await click(rename);
  await settle(10);
  expect(dialogHeading()).toBe("Rename File");
  return $<HTMLInputElement>(".dialog-layer input");
}

test("Rename from the preview: Escape while it runs closes the dialog, and the file is still renamed", async () => {
  const dir = await folder("preview-escape");
  const input = await openPreviewRename("a.txt");
  await type(input, "c.txt");
  calls().hold("rename_entry");
  await key("Enter", { target: input });
  expect(dialogHeading()).toBe("Rename File");
  await key("Escape", { target: input });
  expect(dialogHeading()).toBeUndefined();
  calls().release("rename_entry");
  await settle(30);
  expect(await invoke("path_exists", { path: `${dir}/c.txt` })).toBe(true);
});

test("Rename from the preview: a failure stays in the dialog, cleaned, with everything enabled", async () => {
  await folder("preview-fail");
  const input = await openPreviewRename("a.txt");
  await type(input, "c.txt");
  calls().failNext("rename_entry", "Failed to rename: Permission denied (os error 13)");
  await respond("Rename");
  expect(dialogHeading()).toBe("Rename File");
  expect(message()).toBe("Permission denied");
  expect(button("Rename").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  expect(input.disabled).toBe(false);
});

// ----- A double Enter runs the work once -----

test("Compress: a double Enter compresses once", async () => {
  await folder("compress-twice");
  await openCompress("a.txt");
  calls().hold("compress_archive");
  await key("Enter", { target: $("#compress-name") });
  await key("Enter", { target: $("#compress-name") });
  expect(button("Compress").disabled).toBe(true);
  calls().release("compress_archive");
  await settle(30);
  expect(calls().count("compress_archive")).toBe(1);
  expect(dialogHeading()).toBeUndefined();
  expect(toasts().join(" ")).toContain("Created “a.txt.zip”");
});

test("Batch Rename: a double Enter renames once", async () => {
  await folder("batch-twice");
  await openBatchRename();
  calls().hold("rename_many");
  await key("Enter", { target: $("#batch-prefix") });
  await key("Enter", { target: $("#batch-prefix") });
  expect(button("Rename").disabled).toBe(true);
  calls().release("rename_many");
  await settle(30);
  expect(calls().count("rename_many")).toBe(1);
  expect(dialogHeading()).toBeUndefined();
  expect(toasts().join(" ")).toContain("Renamed 2 items");
  expect(names()).toEqual(expect.arrayContaining(["x-a.txt", "x-b.txt"]));
});

test("New Folder: a double Enter creates once, without an error toast", async () => {
  await folder("new-twice", []);
  const input = await openNewFolder("Made");
  calls().reset();
  calls().hold("make_dir");
  await key("Enter", { target: input });
  await key("Enter", { target: input });
  expect(button("Create").disabled).toBe(true);
  calls().release("make_dir");
  await settle(30);
  expect(calls().count("make_dir")).toBe(1);
  expect(dialogHeading()).toBeUndefined();
  expect(toasts().join(" ")).not.toContain("already exists");
  expect(names()).toEqual(["Made"]);
});

test("Rename from the preview: a double Enter renames once", async () => {
  const dir = await folder("preview-twice");
  const input = await openPreviewRename("a.txt");
  await type(input, "c.txt");
  calls().hold("rename_entry");
  await key("Enter", { target: input });
  await key("Enter", { target: input });
  expect(button("Rename").disabled).toBe(true);
  calls().release("rename_entry");
  await settle(30);
  expect(calls().count("rename_entry")).toBe(1);
  expect(dialogHeading()).toBeUndefined();
  expect(await invoke("path_exists", { path: `${dir}/c.txt` })).toBe(true);
});

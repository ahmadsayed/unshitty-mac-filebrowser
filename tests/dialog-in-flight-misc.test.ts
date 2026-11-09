// The remaining dialogs while their work runs: sharing's name prompt,
// Permissions, New Document and the F2 rename popover. Shared helpers live in
// tests/support/dialog-in-flight.ts.
import { beforeAll, beforeEach, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, chooseMenu, click, dialogHeading, hit, item, key, names, respond, rightClick, settle, toasts, type } from "./app";
import { button, calls, cleanDialogs, folder, message, setupDialogs } from "./support/dialog-in-flight";

beforeAll(setupDialogs);
beforeEach(cleanDialogs);

// ----- What name should people see? -----

test("What name should people see?: a double Enter saves the name once and opens one Share dialog", async () => {
  (globalThis as unknown as { __mockShare: { reset(): void } }).__mockShare.reset();
  const dir = await folder("share-name", []);
  await invoke("make_dir", { path: `${dir}/Named` });
  await key("F5");
  await rightClick(item("Named"));
  await chooseMenu("Share Folder…");
  await settle(10);
  expect(dialogHeading()).toBe("What name should people see?");
  const input = $<HTMLInputElement>("#share-name");
  calls().hold("set_share_name");
  await key("Enter", { target: input });
  await key("Enter", { target: input });
  expect(button("Continue").disabled).toBe(true);
  expect(button("Cancel").disabled).toBe(true);
  expect(input.disabled).toBe(true);
  await key("Escape", { target: input });
  expect(dialogHeading()).toBe("What name should people see?");
  calls().release("set_share_name");
  await settle(30);
  expect(calls().count("set_share_name")).toBe(1);
  expect($$(".dialog-layer").length).toBe(1);
  expect(dialogHeading()).toBe("Share “Named”");
});

test("What name should people see?: a failure stays in the dialog, cleaned, with everything enabled", async () => {
  (globalThis as unknown as { __mockShare: { reset(): void } }).__mockShare.reset();
  const dir = await folder("share-name-fail", []);
  await invoke("make_dir", { path: `${dir}/Named` });
  await key("F5");
  await rightClick(item("Named"));
  await chooseMenu("Share Folder…");
  await settle(10);
  calls().failNext("set_share_name", "Failed to save: Read-only file system (os error 30)");
  await respond("Continue");
  expect(dialogHeading()).toBe("What name should people see?");
  expect(message()).toBe("Read-only file system");
  expect($(".dialog-layer .field-message").classList.contains("error")).toBe(true);
  expect(button("Continue").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  expect($<HTMLInputElement>("#share-name").disabled).toBe(false);
});

// ----- Permissions -----

async function openPermissions(): Promise<void> {
  await folder("perm", ["p.txt"]);
  await rightClick(item("p.txt"));
  await chooseMenu("Properties");
  await settle(20);
  await respond("Permissions…");
  await settle(10);
  expect(dialogHeading()).toBe("Permissions");
}

test("Permissions: a double Enter changes them once and reopens one Properties dialog", async () => {
  await openPermissions();
  calls().hold("set_permissions");
  await key("Enter");
  await key("Enter");
  expect(button("Change").disabled).toBe(true);
  expect(button("Cancel").disabled).toBe(true);
  expect($<HTMLInputElement>('input[data-bit="128"]').disabled).toBe(true);
  await key("Escape");
  expect(dialogHeading()).toBe("Permissions");
  calls().release("set_permissions");
  await settle(40);
  expect(calls().count("set_permissions")).toBe(1);
  expect($$(".dialog-layer").length).toBe(1);
  expect(dialogHeading()).toBe("Properties");
  await key("Escape");
});

test("Permissions: a failure stays in the dialog, cleaned, with everything enabled", async () => {
  await openPermissions();
  calls().failNext("set_permissions", "Failed to set permissions: Operation not permitted (os error 1)");
  await respond("Change");
  expect(dialogHeading()).toBe("Permissions");
  expect(message()).toBe("Operation not permitted");
  expect($(".dialog-layer .field-message").classList.contains("error")).toBe(true);
  expect(button("Change").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  expect($<HTMLInputElement>('input[data-bit="128"]').disabled).toBe(false);
  await respond("Cancel");
  await key("Escape");
});

// ----- New Document: the choices are buttons, and Enter is its Cancel -----

const choice = (label: string) => $$(".dialog-layer .app-choice").find((b) => b.textContent === label) as HTMLButtonElement;

async function openNewDocument(): Promise<void> {
  await folder("newdoc", []);
  await rightClick($("#view"));
  await chooseMenu("New Document…");
  await settle(20);
  expect(dialogHeading()).toBe("New Document");
}

test("New Document: a double click makes one document, and Enter or Escape waits for it", async () => {
  await openNewDocument();
  calls().hold("create_file");
  await click(choice("Empty Text File"));
  await click(choice("Empty Text File"));
  expect(choice("Empty Text File").disabled).toBe(true);
  expect(choice("Letter.odt").disabled).toBe(true);
  expect(button("Cancel").disabled).toBe(true);
  await key("Enter");
  await key("Escape");
  expect(dialogHeading()).toBe("New Document");
  calls().release("create_file");
  await settle(30);
  expect(calls().count("create_file")).toBe(1);
  expect(dialogHeading()).toBeUndefined();
  expect(names()).toEqual(["Untitled.txt"]);
  expect(toasts().join(" ")).toContain("Created “Untitled.txt”");
});

test("New Document: a failure stays in the dialog, cleaned, with every choice enabled", async () => {
  await openNewDocument();
  calls().failNext("copy_entry", "Failed to copy: No space left on device (os error 28)");
  await click(choice("Letter.odt"));
  await settle(20);
  expect(dialogHeading()).toBe("New Document");
  expect(message()).toBe("No space left on device");
  expect($(".dialog-layer .field-message").classList.contains("error")).toBe(true);
  expect(choice("Letter.odt").disabled).toBe(false);
  expect(choice("Empty Text File").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
});

// ----- The F2 rename popover -----

test("Rename popover: a double Enter renames once, with no error", async () => {
  const dir = await folder("popover-twice");
  await click(hit("a.txt"));
  await key("F2");
  const input = $<HTMLInputElement>(".rename-popover input");
  const rename = $<HTMLButtonElement>(".rename-popover button");
  await type(input, "c.txt");
  calls().hold("rename_entry");
  await key("Enter", { target: input });
  await key("Enter", { target: input });
  expect(rename.disabled).toBe(true);
  await type(input, "d.txt");
  expect(rename.disabled).toBe(true);
  await click(rename);
  calls().release("rename_entry");
  await settle(30);
  expect(calls().count("rename_entry")).toBe(1);
  expect(document.querySelector(".rename-popover")).toBeNull();
  expect(await invoke("path_exists", { path: `${dir}/c.txt` })).toBe(true);
  expect(names()).toEqual(["b.txt", "c.txt"]);
});

test("Rename popover: a failure stays in the popover, and Enter tries again", async () => {
  const dir = await folder("popover-fail");
  await click(hit("a.txt"));
  await key("F2");
  const input = $<HTMLInputElement>(".rename-popover input");
  await type(input, "c.txt");
  calls().failNext("rename_entry", "Failed to rename: Permission denied (os error 13)");
  await key("Enter", { target: input });
  await settle(20);
  expect($(".rename-popover .field-message").textContent).toBe("Permission denied");
  expect($<HTMLButtonElement>(".rename-popover button").disabled).toBe(false);
  await key("Enter", { target: input });
  await settle(30);
  expect(calls().count("rename_entry")).toBe(2);
  expect(document.querySelector(".rename-popover")).toBeNull();
  expect(await invoke("path_exists", { path: `${dir}/c.txt` })).toBe(true);
});

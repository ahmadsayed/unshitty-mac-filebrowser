// Copy, move and paste: name clashes (Replace / Skip / Keep Both, apply to
// all), duplicates in the same folder, refusals, and name validation.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, boot, closeOverlays, dialogHeading, goTo, key, names, respond, selectedNames, settle, toasts, type } from "./app";

const HOME = "/Users/me";

/** `<HOME>/T/<dir>` holding a.txt, b.txt and c.txt. */
async function folderOfThree(dir: string): Promise<void> {
  await invoke("make_dir", { path: `${HOME}/T/${dir}` });
  for (const name of ["a.txt", "b.txt", "c.txt"]) {
    await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/T/${dir}/${name}`, overwrite: false });
  }
}

beforeAll(async () => {
  await boot();
  await invoke("make_dir", { path: `${HOME}/T` });
  for (const dir of ["src", "dst", "src2", "dst2", "src3", "dst3", "src4", "dst4"]) await folderOfThree(dir);
});

async function copyAllFrom(dir: string): Promise<void> {
  await goTo(dir);
  await key("a", { mod: true });
  await key("c", { mod: true });
}

test("copying into the same folder makes “(copy)” duplicates", async () => {
  await copyAllFrom(`${HOME}/T/src`);
  await key("v", { mod: true });
  await settle(20);
  expect(names()).toEqual(expect.arrayContaining(["a (copy).txt", "b (copy).txt", "c (copy).txt"]));
  // The duplicates are selected afterwards.
  expect(selectedNames().sort()).toEqual(["a (copy).txt", "b (copy).txt", "c (copy).txt"]);
  await key("a", { mod: true });
  await key("c", { mod: true });
  await key("v", { mod: true });
  await settle(30);
  expect(names()).toContain("a (copy 2).txt");
  expect(names()).toContain("a (copy) (copy).txt");
});

test("a clash asks; Keep Both, Skip and Replace each do their thing", async () => {
  await copyAllFrom(`${HOME}/T/src2`);
  await goTo(`${HOME}/T/dst2`);
  await key("v", { mod: true });
  expect(dialogHeading()).toBe("Replace file “a.txt”?");
  await respond("Keep Both");
  expect(dialogHeading()).toMatch(/Replace file/);
  await respond("Skip");
  expect(dialogHeading()).toMatch(/Replace file/);
  await respond("Replace");
  await settle(20);
  expect(names()).toContain("a (copy).txt");
  expect(names().filter((n) => n.startsWith("b")).length).toBe(1);
});

test("“Apply to all” answers every remaining clash", async () => {
  await copyAllFrom(`${HOME}/T/src3`);
  await goTo(`${HOME}/T/dst3`);
  const before = names().length;
  await key("v", { mod: true });
  const box = document.querySelector<HTMLInputElement>(".dialog-layer .check-row input")!;
  box.checked = true;
  box.dispatchEvent(new Event("change"));
  await respond("Skip");
  await settle(20);
  expect(dialogHeading()).toBeUndefined();
  expect(names().length).toBe(before);
});

test("closing the clash dialog cancels the rest", async () => {
  await copyAllFrom(`${HOME}/T/src4`);
  await goTo(`${HOME}/T/dst4`);
  const before = names().length;
  await key("v", { mod: true });
  await closeOverlays();
  await settle(20);
  expect(names().length).toBe(before);
});

test("cut and paste into its own folder does nothing; a failed move is reported", async () => {
  await goTo(`${HOME}/T/src`);
  await key("Home");
  await key("x", { mod: true });
  const before = names();
  await key("v", { mod: true });
  await settle(20);
  expect(names()).toEqual(before);
  // Cut a folder, then paste it into itself.
  await goTo(`${HOME}/T`);
  await key("Home");
  await key("x", { mod: true });
  await goTo(`${HOME}/T/${names()[0] ?? "dst"}`);
  await key("v", { mod: true });
  await settle(20);
  expect(toasts().join(" ")).toMatch(/Could not move|into itself|nothing/i);
});

test("New Folder validates names like GNOME Files", async () => {
  await goTo(`${HOME}/T`);
  await key("N", { mod: true, shift: true });
  const input = $<HTMLInputElement>("#new-folder-name");
  const message = () => document.querySelector(".dialog-layer .field-message")?.textContent ?? "";
  const createEnabled = () =>
    !document.querySelector<HTMLButtonElement>(".dialog-layer .dialog-responses button:last-child")!.disabled;
  await type(input, "");
  expect(createEnabled()).toBe(false);
  await type(input, "a/b");
  expect(message()).toBe("Folder names cannot contain “/”.");
  await type(input, "..");
  expect(message()).toBe("A folder cannot be called “..”.");
  await type(input, "x".repeat(256));
  expect(message()).toBe("Folder name is too long.");
  await type(input, "src");
  expect(message()).toBe("A folder with that name already exists.");
  await type(input, ".hidden");
  expect(message()).toMatch(/hidden/);
  expect(createEnabled()).toBe(true);
  await type(input, "Made Here");
  await respond("Create");
  expect(names()).toContain("Made Here");
});

test("renaming refuses a clash and renames otherwise", async () => {
  await goTo(`${HOME}/T/src`);
  await key("Home");
  await key("F2");
  const input = document.querySelector<HTMLInputElement>(".rename-popover input")!;
  await type(input, "b.txt");
  expect(document.querySelector(".rename-popover")?.textContent).toMatch(/already exists/);
  await type(input, "renamed.txt");
  await key("Enter", { target: input });
  await settle(15);
  expect(names()).toContain("renamed.txt");
});

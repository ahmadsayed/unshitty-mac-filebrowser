// What each dialog does while its work runs: a second Enter or click, Escape,
// and the buttons after a failure. Each dialog keeps its own answer (Drive and
// Connect to Server keep Cancel live, Share and the installers don't).
import { beforeAll, beforeEach, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import {
  $, $$, boot, chooseMenu, click, closeOverlays, dialogHeading, goTo, hit, item, key, mouse, names, respond, settle, title, toasts, type,
} from "./app";

const HOME = "/Users/me";
const ROOT = `${HOME}/InFlight`;

type MockCalls = {
  count(cmd: string): number; hold(cmd: string): void; release(cmd: string): void;
  failNext(cmd: string, error: string): void; reset(): void;
};
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
const sftp = () => (globalThis as unknown as { __mockSftp: { reset(): void; requirePassword(): void } }).__mockSftp;

const button = (label: string) =>
  $$(".dialog-layer .dialog-responses button").find((b) => b.textContent?.trim() === label) as HTMLButtonElement;
const message = () => $(".dialog-layer .field-message").textContent;

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

let folders = 0;
/** A fresh folder holding `files`, shown in the view. */
async function folder(name: string, files: string[] = ["a.txt", "b.txt"]): Promise<string> {
  const dir = `${ROOT}/${name}-${++folders}`;
  await invoke("make_dir", { path: dir });
  for (const file of files) await invoke("create_file", { path: `${dir}/${file}` });
  await goTo(dir);
  return dir;
}

beforeAll(async () => {
  await invoke("make_dir", { path: ROOT });
  await boot();
  await key("1", { mod: true });
});

beforeEach(async () => {
  calls().reset();
  await closeOverlays();
  if (!$("#preview-layer").hidden) await key("Escape");
});

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

// ----- Install Application (a disk image, on the Mac) -----

async function openDmgInstall(): Promise<void> {
  const dir = await folder("dmg", []);
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${dir}/Flight.dmg`, overwrite: false });
  await key("F5");
  await rightClick(item("Flight.dmg"));
  await chooseMenu("Install Application…");
  expect(dialogHeading()).toBe("Install Application?");
}

test("Install Application: while it runs nothing else responds, and it installs once", async () => {
  await openDmgInstall();
  calls().hold("install_dmg");
  await respond("Install");
  expect(message()).toBe("Installing…");
  expect(button("Install").disabled).toBe(true);
  expect(button("Cancel").disabled).toBe(true);
  expect($<HTMLSelectElement>("#dmg-application").disabled).toBe(true);
  expect($<HTMLSelectElement>("#dmg-destination").disabled).toBe(true);
  await respond("Install");
  await key("Enter");
  await key("Escape");
  expect(dialogHeading()).toBe("Install Application?");
  calls().release("install_dmg");
  await settle(20);
  expect(calls().count("install_dmg")).toBe(1);
  expect(dialogHeading()).toBeUndefined();
  expect(toasts().join(" ")).toContain("Installed “Flight”");
});

test("Install Application: a failure stays in the dialog, word for word, with everything enabled", async () => {
  await openDmgInstall();
  calls().failNext("install_dmg", "Failed: the disk image could not be opened (os error 5)");
  await respond("Install");
  expect(dialogHeading()).toBe("Install Application?");
  expect(message()).toBe("Failed: the disk image could not be opened (os error 5)");
  expect($(".dialog-layer .field-message").classList.contains("error")).toBe(true);
  expect(button("Install").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  expect($<HTMLSelectElement>("#dmg-application").disabled).toBe(false);
  expect($<HTMLSelectElement>("#dmg-destination").disabled).toBe(false);
});

// ----- Connect to Server -----

async function openConnect(): Promise<HTMLInputElement> {
  sftp().reset();
  await click($$(".sidebar-row").find((row) => row.textContent?.trim() === "Network")!);
  await settle(20);
  await rightClick($("#view"));
  await chooseMenu("Connect to Server…");
  const server = $<HTMLInputElement>("#sftp-server");
  await type(server, "user@host");
  return server;
}

test("Connect to Server: a second Enter waits, Escape closes, and the connection still goes ahead", async () => {
  const server = await openConnect();
  calls().hold("sftp_connect");
  await key("Enter", { target: server });
  expect(message()).toBe("Connecting…");
  expect(button("Connect").disabled).toBe(true);
  expect(button("Cancel").disabled).toBe(false);
  await key("Enter", { target: server });
  await key("Escape", { target: server });
  expect(dialogHeading()).toBeUndefined();
  calls().release("sftp_connect");
  await settle(30);
  expect(calls().count("sftp_connect")).toBe(1);
  expect(title()).toBe("user@host");
});

test("Connect to Server: a failure stays in the dialog, cleaned, with everything enabled", async () => {
  const server = await openConnect();
  calls().failNext("sftp_connect", "Failed to connect: Connection refused (os error 111)");
  await respond("Connect");
  expect(dialogHeading()).toBe("Connect to Server");
  expect(message()).toBe("Connection refused");
  expect(button("Connect").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  expect(server.disabled).toBe(false);
  expect($<HTMLInputElement>("#sftp-password").disabled).toBe(false);
});

test("Connect to Server: a refused key moves to the password field", async () => {
  const server = await openConnect();
  sftp().requirePassword();
  await key("Enter", { target: server });
  await settle(20);
  expect(message()).toBe("No SSH key was accepted. Enter your password.");
  expect(document.activeElement).toBe($("#sftp-password"));
  expect(button("Connect").disabled).toBe(false);
});

// ----- Share -----

async function openShare(): Promise<void> {
  await invoke("set_share_name", { name: "Me" });
  const dir = await folder("share", []);
  await invoke("make_dir", { path: `${dir}/Shared` });
  await key("F5");
  await rightClick(item("Shared"));
  await chooseMenu("Share Folder…");
  await settle(10);
  expect(dialogHeading()).toBe("Share “Shared”");
}

const shareButtons = () => ["Copy Link", "Specific Person…", "Stop Sharing…", "Close"].map((label) => button(label).disabled);

test("Share: while a link is made nothing else responds, and one link is made", async () => {
  await openShare();
  calls().hold("create_link");
  await respond("Copy Link");
  expect(shareButtons()).toEqual([true, true, true, true]);
  expect($("#share-protect").matches(":disabled")).toBe(true);
  await respond("Copy Link");
  await key("Enter");
  await key("Escape");
  expect(dialogHeading()).toBe("Share “Shared”");
  calls().release("create_link");
  await settle(20);
  expect(calls().count("create_link")).toBe(1);
  expect(shareButtons()).toEqual([false, false, false, false]);
  expect($("#share-protect").matches(":disabled")).toBe(false);
  expect($$(".share-access-row")).toHaveLength(1);
  expect(toasts().join(" ")).toContain("Link copied");
});

test("Share: a failure is a toast, and everything is enabled again", async () => {
  await openShare();
  calls().failNext("create_link", "Failed to create link: Disk full");
  await respond("Copy Link");
  expect(dialogHeading()).toBe("Share “Shared”");
  expect(toasts().join(" ")).toContain("Could not create the link: Disk full");
  expect(shareButtons()).toEqual([false, false, false, false]);
  expect($("#share-protect").matches(":disabled")).toBe(false);
});

// ----- Google Drive -----

const driveRow = () => $$("#sidebar-places .sidebar-row").find((row) => row.textContent?.includes("Google Drive"))!;
const CLIENT = JSON.stringify({ installed: { client_id: "123-abc.apps.googleusercontent.com", client_secret: "GOCSPX-test" } });
const link = (label: string) => $$(".dialog-layer .dialog-links button").find((b) => b.textContent === label)!;

test("Add Your Google Client: Escape while it's added closes it, and Connect still follows", async () => {
  driveRow().click();
  await settle(10);
  expect(dialogHeading()).toBe("Add Your Google Client");
  $<HTMLTextAreaElement>(".client-json").value = CLIENT;
  calls().hold("drive_client_set");
  await respond("Add Client");
  expect(button("Cancel").disabled).toBe(false);
  await key("Escape");
  expect(dialogHeading()).toBeUndefined();
  calls().release("drive_client_set");
  await settle(20);
  expect(dialogHeading()).toBe("Connect to Google Drive");
});

test("Add Your Google Client: a failure stays in the dialog, cleaned, with everything enabled", async () => {
  await invoke("drive_client_clear");
  driveRow().click();
  await settle(10);
  $<HTMLTextAreaElement>(".client-json").value = CLIENT;
  calls().failNext("drive_client_set", "Invalid client: Use a Desktop app client");
  await respond("Add Client");
  expect(dialogHeading()).toBe("Add Your Google Client");
  expect($(".dialog-error").textContent).toBe("Use a Desktop app client");
  expect(button("Add Client").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  expect($<HTMLTextAreaElement>(".client-json").disabled).toBe(false);
  expect(link("Choose File…").disabled).toBe(false);
});

test("Connect to Google Drive: while it waits Cancel stays live, and Open Browser runs once", async () => {
  await invoke("drive_client_set", { json: CLIENT });
  driveRow().click();
  await settle(10);
  expect(dialogHeading()).toBe("Connect to Google Drive");
  calls().hold("drive_connect_finish");
  await respond("Open Browser");
  expect($(".dialog-layer .dialog-extra .dialog-text").textContent).toBe("Waiting for Google…");
  expect(button("Open Browser").disabled).toBe(true);
  expect(button("Cancel").disabled).toBe(false);
  expect(link("Open Guide").disabled).toBe(false);
  await respond("Open Browser");
  await click(link("Change Client…"));
  expect(calls().count("drive_connect_start")).toBe(1);
  expect(calls().count("drive_client_clear")).toBe(0);
  await key("Escape");
  expect(dialogHeading()).toBeUndefined();
  expect(calls().count("drive_connect_cancel")).toBe(1);
  calls().release("drive_connect_finish");
  await settle(20);
});

test("Connect to Google Drive: a failure is shown in the dialog and as a toast, and Open Browser is enabled", async () => {
  driveRow().click();
  await settle(10);
  calls().failNext("drive_connect_start", "Failed to start: Network is unreachable");
  await respond("Open Browser");
  expect(dialogHeading()).toBe("Connect to Google Drive");
  expect($(".dialog-layer .dialog-extra .dialog-text").textContent).toBe("Network is unreachable");
  expect(toasts().join(" ")).toContain("Network is unreachable");
  expect(button("Open Browser").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  expect(link("Change Client…").disabled).toBe(false);
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

test("Add Your Google Client: a double click adds the client once and opens one Connect dialog", async () => {
  await invoke("drive_client_clear");
  driveRow().click();
  await settle(10);
  expect(dialogHeading()).toBe("Add Your Google Client");
  $<HTMLTextAreaElement>(".client-json").value = CLIENT;
  calls().hold("drive_client_set");
  await respond("Add Client");
  expect(button("Add Client").disabled).toBe(true);
  await respond("Add Client");
  calls().release("drive_client_set");
  await settle(20);
  expect(calls().count("drive_client_set")).toBe(1);
  expect($$(".dialog-layer").length).toBe(1);
  expect(dialogHeading()).toBe("Connect to Google Drive");
});

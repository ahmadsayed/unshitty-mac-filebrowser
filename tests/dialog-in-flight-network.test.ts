// Network and sharing dialogs while their work runs: Install Application,
// Connect to Server, Share and Google Drive. Shared helpers live in
// tests/support/dialog-in-flight.ts.
import { beforeAll, beforeEach, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, chooseAccountRow, chooseMenu, click, dialogHeading, item, key, openOnlineAccounts, respond, rightClick, settle, title, toasts, type } from "./app";
import { HOME, button, calls, cleanDialogs, folder, message, setupDialogs, sftp } from "./support/dialog-in-flight";

beforeAll(setupDialogs);
beforeEach(cleanDialogs);

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
  expect(title()).toBe("user");
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

test("Connect to Server: a failure after Trust gives focus back to the Server field", async () => {
  const server = await openConnect();
  sftp().unknownHost();
  server.focus();
  await key("Enter", { target: server });
  await settle(20);
  expect(dialogHeading()).toBe("Trust This Server?");
  calls().failNext("sftp_connect", "Failed to connect: Connection refused (os error 111)");
  await respond("Trust");
  await settle(20);
  expect(dialogHeading()).toBe("Connect to Server");
  expect(message()).toBe("Connection refused");
  expect(server.disabled).toBe(false);
  expect(document.activeElement).toBe(server);
  await key("Escape", { target: server });
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

/** Online Accounts, then Google Drive under Connect an Account, clicked. */
async function clickConnectDrive(): Promise<void> {
  await openOnlineAccounts();
  await chooseAccountRow("Connect an Account", "Google Drive");
}
const CLIENT = JSON.stringify({ installed: { client_id: "123-abc.apps.googleusercontent.com", client_secret: "GOCSPX-test" } });
const link = (label: string) => $$(".dialog-layer .dialog-links button").find((b) => b.textContent === label)!;

test("Add Your Google Client: Escape while it's added closes it, and Connect still follows", async () => {
  await clickConnectDrive();
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
  await clickConnectDrive();
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
  await clickConnectDrive();
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
  await clickConnectDrive();
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

// ----- A double click runs the work once -----

test("Add Your Google Client: a double click adds the client once and opens one Connect dialog", async () => {
  await invoke("drive_client_clear");
  await clickConnectDrive();
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

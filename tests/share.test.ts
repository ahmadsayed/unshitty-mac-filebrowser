// Network, the Share dialog and read-only share browsing. The backend is the
// in-memory mock; __mockShare changes the fake sharer's state.
import { beforeAll, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import {
  $, $$, boot, chooseMenu, click, closeOverlays, dialogHeading, goTo, item, key, menuLabels,
  mouse, names, open, respond, settle, title, toasts, type,
} from "./app";

const HOME = "/Users/me";
const place = (label: string) => $$(".sidebar-row").find((row) => row.textContent?.trim() === label)!;

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

function resetShareMock(): void {
  (globalThis as unknown as { __mockShare: { reset(): void; setOnline(v: boolean): void } }).__mockShare.reset();
}

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

test("Network is empty with a paste bar and reports a bad link inline", async () => {
  resetShareMock();
  await click(place("Network"));
  await settle(10);
  expect(title()).toBe("Network");
  expect($("#view").textContent).toContain("No Network Locations");
  expect($("#view").textContent).toContain("Paste a share link below to add a shared folder.");
  const input = $<HTMLInputElement>(".network-link");
  expect(input).toBeTruthy();

  await type(input, "not a link");
  await click($(".network-paste .btn"));
  await settle(10);
  expect($(".network-error").textContent).toContain("This isn't a share link.");

  await type(input, "fbshare1:mock-duplicate");
  await click($(".network-paste .btn"));
  await settle(20);
  await type($<HTMLInputElement>(".network-link"), "fbshare1:mock-duplicate");
  await click($(".network-paste .btn"));
  await settle(20);
  expect($(".network-error").textContent).toContain("already in Network");
});

test("sharing asks for a name, copies links, and a person link starts unused", async () => {
  resetShareMock();
  await goTo(HOME);
  await rightClick(item("main.rs"));
  expect(menuLabels()).not.toContain("Share…");
  await settle();
  await rightClick(item("Documents"));
  expect(menuLabels()).toContain("Share…");
  await chooseMenu("Share…");
  await settle(10);

  expect(dialogHeading()).toBe("What name should people see?");
  expect($<HTMLInputElement>("#share-name").value).toBe("me");
  await type($<HTMLInputElement>("#share-name"), "Ahmed");
  await respond("Continue");
  await settle(20);
  expect(dialogHeading()).toBe("Share “Documents”");

  await respond("Copy Link");
  await settle(25);
  expect($(".share-access-list").textContent).toContain("Anyone with the link");
  expect($(".share-access-list").textContent).toContain("Hasn't opened the link yet");
  const clip = vi.mocked(navigator.clipboard.writeText);
  const anyoneLink = String(clip.mock.calls.at(-1)?.[0]);
  expect(anyoneLink).toMatch(/^fbshare1:/);

  await respond("Specific Person…");
  await settle(10);
  expect(dialogHeading()).toBe("Share With a Specific Person");
  await type($<HTMLInputElement>("#share-person"), "Sara");
  await respond("Create Link");
  await settle(25);
  expect($(".share-access-list").textContent).toContain("Sara");
  expect($(".share-access-list").textContent).toContain("Hasn't opened the link yet");

  await respond("Close");
  await settle(10);
  expect(await invoke<unknown[]>("list_shares")).toHaveLength(1);
});

test("a received share browses read-only and remembers an offline sharer", async () => {
  resetShareMock();
  const received = await invoke<{ id: string }>("add_received", { link: "fbshare1:mock-received" });
  await click(place("Network"));
  await settle(20);
  expect(names()).toEqual(["Photos"]);
  expect($$(".shared-by-cell").map((cell) => cell.textContent)).toEqual(["Ahmed"]);

  await open("Photos");
  await settle(20);
  expect(title()).toBe("Photos");
  expect($("#share-subtitle").textContent).toBe("Shared by Ahmed · Read only");
  expect(names()).toEqual(["Photos", "hello.txt"]);

  await rightClick(item("hello.txt"));
  const labels = menuLabels();
  expect(labels).toContain("Open");
  expect(labels).toContain("Copy");
  expect(labels).toContain("Properties");
  expect(labels).not.toContain("Cut");
  expect(labels).not.toContain("Rename…");
  expect(labels).not.toContain("Move to Trash");
  expect(labels).not.toContain("Star");
  await settle();
  await rightClick(item("Photos"));
  expect(menuLabels()).not.toContain("Share…");
  await closeOverlays();

  await rightClick($("#view"));
  expect(menuLabels()).toEqual(expect.arrayContaining(["Show Hidden Files", "Reload", "Select All"]));
  expect(menuLabels()).not.toContain("New Folder…");
  expect(menuLabels()).not.toContain("Paste");

  (globalThis as unknown as { __mockShare: { setOnline(v: boolean): void } }).__mockShare.setOnline(false);
  await click(place("Network"));
  await settle(20);
  expect($$(".network-status").map((cell) => cell.textContent)).toEqual(["Offline"]);
  await open("Photos");
  await settle(20);
  expect($("#view").textContent).toContain("Ahmed Is Offline");
  expect($("#view").textContent).toContain(
    "Shared folders open while the computer sharing them is on and Files is running."
  );

  (globalThis as unknown as { __mockShare: { setOnline(v: boolean): void } }).__mockShare.setOnline(true);
  expect(received.id).toBeTruthy();
});

test("Share This Folder… is in the path-bar menu only for local folders", async () => {
  resetShareMock();
  await goTo(HOME);
  await click($("#btn-location-menu"));
  await settle();
  expect(menuLabels()).toContain("Share This Folder…");
  await closeOverlays();

  await invoke("add_received", { link: "fbshare1:mock-pathbar" });
  await click(place("Network"));
  await settle(20);
  await open("Photos");
  await settle(20);
  await click($("#btn-location-menu"));
  await settle();
  const labels = menuLabels();
  expect(labels).toContain("Copy Location");
  expect(labels).toContain("Reload");
  expect(labels).not.toContain("Share This Folder…");
  expect(labels).not.toContain("New Folder…");
  expect(labels).not.toContain("Paste");
});

test("pasting your own link opens the local folder", async () => {
  resetShareMock();
  await goTo(HOME);
  await rightClick(item("Documents"));
  await chooseMenu("Share…");
  await settle(10);
  await type($<HTMLInputElement>("#share-name"), "Ahmed");
  await respond("Continue");
  await settle(20);
  await respond("Copy Link");
  await settle(25);
  const own = String(vi.mocked(navigator.clipboard.writeText).mock.calls.at(-1)?.[0]);
  expect(own).toMatch(/^fbshare1:own-/);
  await respond("Close");
  await settle(10);

  await click(place("Network"));
  await settle(20);
  await type($<HTMLInputElement>(".network-link"), own);
  await click($(".network-paste .btn"));
  await settle(40);
  expect(title()).toBe("Documents");
  expect(toasts().join(" | ")).toContain("This is your own shared folder.");
});

test("a share link typed in the path bar opens the share", async () => {
  resetShareMock();
  await goTo(HOME);
  await goTo("fbshare1:mock-path-entry");
  await settle(30);
  expect(title()).toBe("Photos");
  expect($("#share-subtitle").textContent).toBe("Shared by Ahmed · Read only");
  expect(names()).toEqual(["Photos", "hello.txt"]);
});

test("losing access shows the No Access page", async () => {
  resetShareMock();
  await invoke("add_received", { link: "fbshare1:mock-noaccess" });
  (globalThis as unknown as { __mockShare: { setNoAccess(v: boolean): void } }).__mockShare.setNoAccess(true);
  await click(place("Network"));
  await settle(20);
  expect($$(".network-status").map((cell) => cell.textContent)).toEqual(["No access"]);
  await open("Photos");
  await settle(20);
  expect($("#view").textContent).toContain("No Access");
  expect($("#view").textContent).toContain("Ahmed stopped sharing this folder with you.");
  (globalThis as unknown as { __mockShare: { setNoAccess(v: boolean): void } }).__mockShare.setNoAccess(false);
});

test("Stop Sharing confirms with Cancel as the default, and Remove from Network forgets it", async () => {
  resetShareMock();
  await goTo(HOME);
  await rightClick(item("Documents"));
  await chooseMenu("Share…");
  await settle(10);
  await type($<HTMLInputElement>("#share-name"), "Ahmed");
  await respond("Continue");
  await settle(20);

  await respond("Stop Sharing…");
  await settle(10);
  expect(dialogHeading()).toBe("Stop sharing “Documents”?");
  expect($$(".dialog-layer").at(-1)!.textContent).toContain(
    "People you shared it with will no longer be able to open it."
  );
  await respond("Cancel");
  await settle(10);
  expect(await invoke<unknown[]>("list_shares")).toHaveLength(1);
  await respond("Close");
  await settle(10);

  await invoke("add_received", { link: "fbshare1:mock-remove" });
  await click(place("Network"));
  await settle(20);
  await rightClick(item("Photos"));
  await chooseMenu("Remove from Network");
  await settle(20);
  expect(names()).toEqual([]);
  expect($("#view").textContent).toContain("No Network Locations");
});

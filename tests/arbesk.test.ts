// Arbesk in Network: connect with an email through the mock, browse the
// account's collections and assets read only, and keep its row when the
// backend can't answer for a moment.
import { beforeAll, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, accountGroups, boot, chooseAccountRow, click, closeOverlays, dialogHeading, goTo, installFakeCanvas, key, menuLabels, mouse, names, open, openOnlineAccounts, respond, settle, toasts, type } from "./app";

const calls = () =>
  (globalThis as unknown as { __mockCalls: { failNext(cmd: string, error: string): void } }).__mockCalls;

beforeAll(async () => {
  installFakeCanvas(); // the mock draws the thumbnails Arbesk hands over
  await boot();
  await key("1", { mod: true }); // the list view, with its size and date cells
});

async function openNetwork(): Promise<void> {
  await click($$(".sidebar-row").find((row) => row.textContent?.trim() === "Network")!);
  await settle(20);
}

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

const row = (name: string) => $$("#view .item").find((el) => el.querySelector(".name")?.textContent === name)!;
const crumbs = () => $$(".crumb").map((el) => el.textContent?.trim());

test("Arbesk connects with an email and opens on the account's collections", async () => {
  await openOnlineAccounts();
  await chooseAccountRow("Connect an Account", "Arbesk");
  expect(dialogHeading()).toBe("Connect to Arbesk");
  expect($(".dialog-text").textContent).toBe("Arbesk emails you a code. Enter it in your browser to sign in.");
  const email = $<HTMLInputElement>("#arbesk-email");
  const openBrowser = $$(".dialog button").find((b) => b.textContent === "Open Browser")!;
  expect(openBrowser.hasAttribute("disabled")).toBe(true);
  await type(email, "me@example.com");
  expect(openBrowser.hasAttribute("disabled")).toBe(false);

  await respond("Open Browser");
  await settle(20);
  expect(dialogHeading()).toBeUndefined();
  expect(toasts()).toContain("Connected to me@example.com");
  expect(crumbs()).toEqual(["Arbesk"]);
  expect(names().sort()).toEqual(["Shapes", "Untitled Collection"]);
});

test("Network lists the account as Arbesk, and Online Accounts lists it with its email", async () => {
  await openNetwork();
  expect(names()).toContain("Arbesk");
  await openOnlineAccounts();
  expect(accountGroups()[0]).toEqual({ title: "Your Accounts", rows: ["Arbesk · me@example.com"] });
  await closeOverlays();
  // An arbesk:// address means nothing outside the app: no Copy Location.
  await rightClick(row("Arbesk"));
  expect(menuLabels()).toEqual(["Open", "Disconnect…"]);
  await closeOverlays();
});

test("a collection lists its assets as files, without sizes", async () => {
  await openNetwork();
  await open("Arbesk");
  await settle(20);
  await open("Shapes");
  await settle(20);
  expect(crumbs()).toEqual(["Arbesk", "Shapes"]);
  expect(names().sort()).toEqual(["Big.gltf", "Box.3mf", "Box.gltf"]);
  expect(row("Box.gltf").querySelector(".meta.size")?.textContent).toBe("");
  await click(row("Box.3mf"));
  expect($("#floating-bar").textContent).toBe("“Box.3mf” selected");
});

test("an asset shows the thumbnail it was saved with, and one saved without keeps its icon", async () => {
  expect(crumbs()).toEqual(["Arbesk", "Shapes"]);
  await settle(20);
  expect(row("Box.gltf").querySelector(".icon img")).not.toBeNull();
  expect(row("Box.3mf").querySelector(".icon img")).toBeNull();
  expect(row("Big.gltf").querySelector(".icon img")).toBeNull();
});

test("nothing in Arbesk can be written, renamed or copied out", async () => {
  await click(row("Box.gltf"));
  await rightClick(row("Box.gltf"));
  expect(menuLabels()).toEqual(["Open", "Copy"]);
  expect($$(".popover .menu-item").find((el) => el.textContent?.trim() === "Copy")?.hasAttribute("disabled")).toBe(true);
  await closeOverlays();

  await click(row("Box.gltf"));
  await key("c", { mod: true });
  await settle();
  expect(toasts()).toContain("Copying from Arbesk comes next.");

  // A local file on the clipboard can't be pasted into a collection.
  await goTo("/Users/me/Desktop");
  await click(row("todo.txt"));
  await key("c", { mod: true });
  await openNetwork();
  await open("Arbesk");
  await settle(20);
  await open("Shapes");
  await settle(20);
  await key("v", { mod: true });
  await settle();
  expect(toasts()).toContain("Arbesk is read only.");
  expect(names().sort()).toEqual(["Big.gltf", "Box.3mf", "Box.gltf"]);
});

test("Open on an asset opens it in Arbesk's Studio, in the browser", async () => {
  await openNetwork();
  await open("Arbesk");
  await settle(20);
  await open("Shapes");
  await settle(20);
  await click($("#btn-location-menu"));
  expect(menuLabels()).not.toContain("Copy Location");
  await closeOverlays();

  const log = vi.spyOn(console, "log");
  await open("Box.gltf");
  await settle(20);
  expect(log).toHaveBeenCalledWith("mock open_url", "https://promptscad.com/studio?asset=1&assetId=box-g");
  log.mockRestore();
});

test("a failed accounts answer keeps the Arbesk row in Network", async () => {
  calls().failNext("arbesk_accounts", "offline");
  await openNetwork();
  expect(names()).toContain("Arbesk");
});

test("Disconnect… removes the Arbesk account and leaves its collections alone", async () => {
  await openNetwork();
  await rightClick(row("Arbesk"));
  await click($$(".popover .menu-item").find((el) => el.textContent?.trim() === "Disconnect…")!);
  expect(dialogHeading()).toBe("Disconnect Arbesk?");
  expect($(".dialog-text").textContent).toBe("“me@example.com” is removed from File Browser. Your collections on Arbesk are not changed.");
  await respond("Disconnect");
  await settle(20);
  expect(names()).not.toContain("Arbesk");
  expect(await invoke("arbesk_accounts", {})).toEqual([]);
});

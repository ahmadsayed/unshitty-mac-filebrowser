// Share with QR Code: the menu item on one local file, the dialog's code,
// link and status as a phone downloads, and stopping when it closes.
import { beforeAll, expect, test } from "vitest";
import { fileShareMock } from "../dev/mock-tauri/core";
import { downloadsText, statusText } from "../src/qr-share";
import { $, $$, boot, chooseMenu, click, closeOverlays, dialogHeading, goTo, hit, key, menuLabels, mouse, respond, settle, toasts } from "./app";

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

/** Let the dialog poll (every 500 ms) at least once more. */
const poll = async () => {
  await new Promise((r) => setTimeout(r, 550));
  await settle();
};

beforeAll(boot);

test("files offer Share with QR Code…, alone or together; folders don't", async () => {
  await goTo("/Users/me/Documents");
  await rightClick(hit("Notes.md"));
  expect(menuLabels()).toContain("Share with QR Code…");
  await closeOverlays();
  await rightClick(hit("Projects"));
  expect(menuLabels()).not.toContain("Share with QR Code…");
  await closeOverlays();
  await click(hit("Notes.md"));
  await click(hit("Budget 2026.xlsx"), { metaKey: true });
  await rightClick(hit("Budget 2026.xlsx"));
  expect(menuLabels()).toContain("Share with QR Code…");
  await closeOverlays();
  await click(hit("Notes.md"));
  await click(hit("Projects"), { metaKey: true });
  await rightClick(hit("Notes.md"));
  expect(menuLabels()).not.toContain("Share with QR Code…");
  await closeOverlays();
});

test("several files share as one code", async () => {
  await goTo("/Users/me/Documents");
  await click(hit("Notes.md"));
  await click(hit("Budget 2026.xlsx"), { metaKey: true });
  await rightClick(hit("Budget 2026.xlsx"));
  await chooseMenu("Share with QR Code…");
  await settle();
  expect(dialogHeading()).toBe("Share 2 Files");
  expect($(".qr-share-hint").textContent).toMatch(/download the files\./);
  const [share] = [...fileShareMock.shares.values()];
  expect(share.paths.sort()).toEqual(["/Users/me/Documents/Budget 2026.xlsx", "/Users/me/Documents/Notes.md"]);
  await respond("Stop Sharing");
  expect(fileShareMock.shares.size).toBe(0);
});

test("the dialog prepares, shows the code and link, then counts downloads", async () => {
  await goTo("/Users/me/Documents");
  await rightClick(hit("Notes.md"));
  await chooseMenu("Share with QR Code…");
  await settle();
  expect(dialogHeading()).toBe("Share “Notes.md”");
  expect($(".qr-share-hint").textContent).toMatch(/download the file\./);
  expect($(".qr-share-status").textContent).toBe("Preparing… 50%");
  expect($(".qr-share-link").hidden).toBe(true);

  await poll();
  const [id] = [...fileShareMock.shares.keys()];
  const img = $<HTMLImageElement>(".qr-share-image");
  expect(img.getAttribute("src")).toMatch(/^data:image\/svg\+xml;charset=utf-8,/);
  expect($<HTMLInputElement>(".qr-share-link input").value).toBe(`https://example.invalid/get/#${id}`);
  expect($(".qr-share-status").textContent).toBe("Waiting for a phone…");

  fileShareMock.shares.get(id)!.sending = 0.42;
  await poll();
  expect($(".qr-share-status").textContent).toBe("Sending… 42%");
  Object.assign(fileShareMock.shares.get(id)!, { sending: null, downloads: 1 });
  await poll();
  expect($(".qr-share-status").textContent).toBe("Downloaded by 1 device");

  await click($$(".qr-share-link button")[0]);
  await settle();
  expect(navigator.clipboard.writeText).toHaveBeenLastCalledWith(`https://example.invalid/get/#${id}`);
  expect(toasts()).toContain("Link copied");

  await respond("Stop Sharing");
  expect($$(".dialog-layer")).toHaveLength(0);
  expect(fileShareMock.shares.has(id)).toBe(false);
});

test("Escape stops sharing too", async () => {
  await goTo("/Users/me/Documents");
  await rightClick(hit("Notes.md"));
  await chooseMenu("Share with QR Code…");
  await settle();
  expect(fileShareMock.shares.size).toBe(1);
  await key("Escape");
  await settle();
  expect($$(".dialog-layer")).toHaveLength(0);
  expect(fileShareMock.shares.size).toBe(0);
});

test("a failure is shown in the dialog, without a code", async () => {
  await goTo("/Users/me/Documents");
  await rightClick(hit("Notes.md"));
  await chooseMenu("Share with QR Code…");
  await settle();
  const [id] = [...fileShareMock.shares.keys()];
  fileShareMock.shares.get(id)!.error = "Could not reach the relay server. Check the internet connection and try again.";
  await poll();
  expect($(".qr-share-status").textContent).toMatch(/Could not reach the relay server/);
  expect($(".qr-share-status").classList.contains("error")).toBe(true);
  expect($(".qr-share-code").hidden).toBe(true);
  expect($(".qr-share-hint").hidden).toBe(true);
  await respond("Stop Sharing");
  expect(fileShareMock.shares.size).toBe(0);
});

test("a share stopped elsewhere closes its dialog", async () => {
  await goTo("/Users/me/Documents");
  await rightClick(hit("Notes.md"));
  await chooseMenu("Share with QR Code…");
  await settle();
  fileShareMock.shares.clear();
  await poll();
  expect($$(".dialog-layer")).toHaveLength(0);
});

test("status lines read naturally", () => {
  expect(downloadsText(0)).toBe("Waiting for a phone…");
  expect(downloadsText(3)).toBe("Downloaded by 3 devices");
  expect(statusText({ state: "preparing", progress: 1 })).toBe("Connecting…");
  expect(statusText({ state: "ready", link: "", qr: "", downloads: 2, sending: null })).toBe("Downloaded by 2 devices");
});

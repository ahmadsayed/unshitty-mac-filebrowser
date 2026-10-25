// Space opens the preview panel for a text file and closes it again.
import { beforeAll, expect, test } from "vitest";
import { $, boot, click, goTo, item, key, settle } from "./app";

const HOME = "/Users/me";
const pause = (ms = 260) => new Promise((resolve) => setTimeout(resolve, ms));

// The mock thumbnailer draws PDFs with fillRect only.
class FakeCanvas {
  getContext() {
    return { fillStyle: "", fillRect() {} };
  }
  convertToBlob() {
    return Promise.resolve({ arrayBuffer: async () => new ArrayBuffer(8) });
  }
}
(globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeCanvas;

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

test("Space previews the selected text file and Escape closes it", async () => {
  await goTo(`${HOME}/Documents`);
  await click(item("Notes.md"));
  await key(" ");
  await settle(20);
  expect($("#preview-layer").hidden).toBe(false);
  expect($("#preview-name").textContent).toBe("Notes.md");
  expect($("#preview-body").textContent).toContain("Preview of");

  await key("Escape");
  await settle(10);
  expect($("#preview-layer").hidden).toBe(true);
  expect($("#preview-body").textContent).toBe("");
});

test("Space closes an open preview", async () => {
  await key(" ");
  await settle(20);
  expect($("#preview-layer").hidden).toBe(false);
  await key(" ");
  await settle(10);
  expect($("#preview-layer").hidden).toBe(true);
});

test("a PDF is previewed from its thumbnail", async () => {
  await click(item("Quarterly report.pdf"));
  await key(" ");
  await pause();
  await settle(10);
  expect($("#preview-layer").hidden).toBe(false);
  const image = $("#preview-body img");
  expect(image.getAttribute("src")).toBe("blob:test");
  await key("Escape");
});

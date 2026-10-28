// Loading thumbnails in the view: thumbnails of a folder left behind are
// cancelled so the next folder's start at once, and visible rows come before
// the overscan rows rendered around them. The mock holds thumbnails open,
// standing in for a slow Quick Look.
import { beforeAll, beforeEach, expect, test } from "vitest";
import { $, $$, boot, goTo, installFakeCanvas, item, settle } from "./app";

type MockThumbs = { hold(v: boolean): void; held(): number; reset(): void; log: { requested: string[]; cancelled: number[] } };
const thumbs = () => (globalThis as unknown as { __mockThumbs: MockThumbs }).__mockThumbs;

const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));
const CAMERA = "/Users/me/Pictures/Camera";
const SCREENSHOTS = "/Users/me/Pictures/Screenshots";

beforeAll(async () => {
  installFakeCanvas();
  await boot();
});

beforeEach(() => {
  thumbs().reset();
});

test("leaving a folder cancels its thumbnails and starts the next folder's at once", async () => {
  thumbs().hold(true);
  await goTo(CAMERA);
  expect(thumbs().held()).toBe(4);

  await goTo(SCREENSHOTS);
  expect(thumbs().log.cancelled).toHaveLength(4);
  expect(thumbs().held()).toBe(3);
  const shots = ["Screenshot 1.png", "Screenshot 2.png", "Screenshot 3.png"].map((name) => `${SCREENSHOTS}/${name}`);
  expect(thumbs().log.requested.slice(-3).sort()).toEqual(shots);

  // The mock takes 150 ms to draw a picture once released.
  thumbs().hold(false);
  await pause(200);
  expect(item("Screenshot 1.png").querySelector(".icon img")).not.toBeNull();
});

test("after scrolling, visible rows are asked for before the overscan above them", async () => {
  thumbs().hold(true);
  await goTo(CAMERA);
  thumbs().log.requested.length = 0;
  const scroller = $("#view-scroll");
  scroller.scrollTop = 3000;
  scroller.dispatchEvent(new Event("scroll"));
  await pause(50);
  await settle();

  const top = (el: HTMLElement) => parseFloat(el.style.top);
  const rows = [...new Set($$("#view .item[data-index]").map(top))].sort((a, b) => a - b);
  // The first two rendered rows are overscan above the viewport.
  const firstVisibleRow = rows[2];
  const started = thumbs().log.requested.slice(0, 4);
  expect(started).toHaveLength(4);
  for (const path of started) {
    expect(top(item(path.slice(CAMERA.length + 1)))).toBeGreaterThanOrEqual(firstVisibleRow);
  }
  thumbs().hold(false);
  await settle();
});

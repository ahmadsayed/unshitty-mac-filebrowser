#!/usr/bin/env node
// Screenshot the app in the mock preview, for a PR's before/after images.
// Needs `npm run dev:mock` running (port 1420) and Playwright installed
// (`npm install --no-save playwright && npx playwright install chromium`).
//
//   node scripts/mock-screenshot.mjs /Users/me/Pictures after.png
//   node scripts/mock-screenshot.mjs /Users/me/Pictures after-dark.png --dark
//   node scripts/mock-screenshot.mjs /Users/me menu.png --theme gnome --scene menu
//
// Options: --dark, --url <base> (default http://localhost:1420),
// --size <width>x<height> (default 1100x720), --theme gnome|macos (default
// macos; the theme follows the platform, so gnome loads the page with a
// Linux user agent and the app takes its Linux code paths), --scene
// grid|list|menu|dialog (default grid: the folder as it opens; list: list
// view with the second item selected; menu: the main menu open; dialog: the
// Keyboard Shortcuts dialog). Uses ?demo (no test-only folders). The page's
// clock is fixed, so the same scene always gives the same image.
import { createRequire } from "node:module";

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const option = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
};
const VALUE_OPTIONS = ["--url", "--size", "--theme", "--scene"];
const [path, out] = args.filter((a, i) => !a.startsWith("--") && !VALUE_OPTIONS.includes(args[i - 1]));
if (!path || !out) {
  console.error("usage: node scripts/mock-screenshot.mjs <path in the mock> <out.png> [--dark] [--url <base>] [--size WxH] [--theme gnome|macos] [--scene grid|list|menu|dialog]");
  process.exit(2);
}

let chromium;
try {
  ({ chromium } = await import("playwright"));
} catch {
  // ESM imports ignore NODE_PATH; require doesn't, which finds a global
  // Playwright such as the cloud environment's.
  try {
    ({ chromium } = createRequire(import.meta.url)("playwright"));
  } catch {}
}
if (!chromium) {
  console.error("Playwright is missing: npm install --no-save playwright && npx playwright install chromium");
  process.exit(1);
}

const [width, height] = option("--size", "1100x720").split("x").map(Number);
const base = option("--url", "http://localhost:1420");
const theme = option("--theme", "macos");
const USER_AGENTS = {
  macos: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15",
  gnome: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15",
};
if (!USER_AGENTS[theme]) {
  console.error(`unknown theme ${theme}: use gnome or macos`);
  process.exit(2);
}
const MOD = theme === "macos" ? "Meta" : "Control";
const scene = option("--scene", "grid");
const SCENES = {
  grid: async () => {},
  list: async (page) => {
    await page.keyboard.press(`${MOD}+1`);
    await page.waitForTimeout(300);
    await page.locator("#view .item").nth(1).click();
  },
  menu: async (page) => {
    await page.click("#btn-main-menu");
  },
  dialog: async (page) => {
    await page.click("#btn-main-menu");
    await page.getByText("Keyboard Shortcuts", { exact: true }).click();
  },
};
if (!SCENES[scene]) {
  console.error(`unknown scene ${scene}: use ${Object.keys(SCENES).join(", ")}`);
  process.exit(2);
}
const browser = await chromium.launch();
try {
  const page = await browser.newPage({
    viewport: { width, height },
    deviceScaleFactor: 2,
    colorScheme: flag("--dark") ? "dark" : "light",
    userAgent: USER_AGENTS[theme],
  });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  // The mock dates its files relative to now; a fixed clock makes captures
  // taken minutes apart comparable pixel for pixel.
  await page.clock.setFixedTime(new Date("2026-10-28T12:00:00"));
  await page.goto(`${base}/?demo&path=${encodeURIComponent(path)}`);
  await page.waitForSelector("#view .item, #view .status-page", { timeout: 10_000 });
  // Let thumbnails and counts arrive (the mock answers in ~150 ms).
  await page.waitForTimeout(1000);
  await SCENES[scene](page);
  // Let menus, dialogs and the new view settle.
  if (scene !== "grid") await page.waitForTimeout(500);
  await page.screenshot({ path: out });
  console.log(`Saved ${out}`);
  if (errors.length) {
    console.error(`Page errors:\n${errors.join("\n")}`);
    process.exitCode = 1;
  }
} finally {
  await browser.close();
}

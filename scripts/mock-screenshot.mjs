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
// grid|list|menu|item-menu|background-menu|dialog|search|search-list|open-with|qr-share|preview|preview-loading|network-drive|drive-expired|online-accounts|network-address-help|arbesk-connect|arbesk-collection|arbesk-grid|network-arbesk|arbesk-preview (default grid: the folder as it
// opens; list: list view with the second item selected; menu: the main menu
// open; item-menu: the right-click menu of --item; background-menu: the
// right-click menu of the empty area; dialog: the Keyboard Shortcuts dialog; search, search-list: a search
// for --query, default "e", once its results are in; open-with: Open With… on
// --item, default "Notes.md", with --query typed into its search; qr-share:
// Send with QR Code… on --item, once its code shows; preview: Space on
// --item; network-drive: Network with a connected Google account; drive-expired:
// Google Drive whose sign-in expired, once its dialog shows; online-accounts:
// Online Accounts with the mock's Google account connected;
// network-address-help: Network's server address popover open;
// preview-loading: Space on --item while the mock holds its bytes;
// arbesk-connect: Connect to Arbesk with an email typed; arbesk-collection:
// the mock's Arbesk collection as a list; arbesk-grid: the same collection
// in the grid view, with the thumbnails its assets were saved with;
// network-arbesk: Network with the
// Arbesk account; arbesk-preview: Space on --item there, default Box.3mf). Uses ?demo (no
// test-only folders). The page's clock is fixed, so the same scene always
// gives the same image.
import { createRequire } from "node:module";

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const option = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
};
const VALUE_OPTIONS = ["--url", "--size", "--theme", "--scene", "--query", "--item"];
const [path, out] = args.filter((a, i) => !a.startsWith("--") && !VALUE_OPTIONS.includes(args[i - 1]));
if (!path || !out) {
  console.error("usage: node scripts/mock-screenshot.mjs <path in the mock> <out.png> [--dark] [--url <base>] [--size WxH] [--theme gnome|macos] [--scene grid|list|menu|item-menu|background-menu|dialog|search|search-list|open-with|qr-share|preview|preview-loading|network-drive|drive-expired|online-accounts|network-address-help|arbesk-connect|arbesk-collection|arbesk-grid|network-arbesk|arbesk-preview] [--query <text>] [--item <name>]");
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
  "item-menu": async (page) => {
    await page.locator("#view .item").filter({ hasText: option("--item", "Notes.md") }).click({ button: "right" });
  },
  "background-menu": async (page) => {
    // Bottom-right of the view, below and beside the items.
    const box = await page.locator("#view").boundingBox();
    await page.mouse.click(box.x + box.width - 40, box.y + box.height - 40, { button: "right" });
  },
  dialog: async (page) => {
    await page.click("#btn-main-menu");
    await page.getByText("Keyboard Shortcuts", { exact: true }).click();
  },
  search: (page) => search(page),
  "open-with": async (page) => {
    await page.locator("#view .item").filter({ hasText: option("--item", "Notes.md") }).click({ button: "right" });
    await page.getByText("Open With…", { exact: true }).click();
    await page.waitForSelector(".app-choice, .app-loading, .app-status:not([hidden])");
    if (args.includes("--query")) await page.keyboard.type(option("--query", ""));
  },
  preview: async (page) => {
    await page.locator("#view .item").filter({ hasText: option("--item", "Notes.md") }).click();
    await page.keyboard.press("Space");
    await page.waitForSelector("#preview-layer:not([hidden])");
    // A 3D model loads its viewer first, then draws.
    await page.waitForSelector("#preview-body > :not(.preview-loading)");
    await page.waitForTimeout(400);
  },
  // Space on --item while its preview is still loading (the mock holds it).
  "preview-loading": async (page) => {
    await page.evaluate(() => window.__mockCalls.hold("preview_bytes"));
    await page.locator("#view .item").filter({ hasText: option("--item", "Notes.md") }).click();
    await page.keyboard.press("Space");
    await page.waitForSelector("#preview-body .preview-loading");
  },
  "qr-share": async (page) => {
    await page.locator("#view .item").filter({ hasText: option("--item", "Notes.md") }).click({ button: "right" });
    await page.getByText("Send with QR Code…", { exact: true }).click();
    await page.waitForSelector(".qr-share-image");
  },
  "search-list": async (page) => {
    await page.keyboard.press(`${MOD}+1`);
    await page.waitForTimeout(300);
    await search(page);
  },
  // Network (open it with the path pseudo:network) with a Google account
  // connected through the mock, as a list, so the Status column shows.
  "network-drive": async (page) => {
    await connectMockDrive(page);
    await page.locator(".sidebar-row", { hasText: "Network" }).click();
    await page.waitForSelector("#view .item:has-text('Google Drive')");
    await page.keyboard.press(`${MOD}+1`);
    await page.waitForTimeout(300);
  },
  // Online Accounts from the primary menu, with the mock's Google account
  // connected, so both groups show.
  "online-accounts": async (page) => {
    await connectMockDrive(page);
    await page.locator(".toast .flat-btn").click().catch(() => {});
    await page.locator("#btn-main-menu").click();
    await page.getByRole("menuitem", { name: "Online Accounts…", exact: true }).click();
    await page.waitForSelector(".dialog .preferences-group");
  },
  // Network's info button: the server addresses the bar takes.
  "network-address-help": async (page) => {
    await page.locator(".network-paste .network-info").click();
    await page.waitForSelector(".popover .server-help");
  },
  // The mock's Google Drive sign-in has expired: opening the entry shows the
  // dialog that fixes it (or, on a build before it, the old toast).
  "drive-expired": async (page) => {
    await connectMockDrive(page);
    await page.locator(".sidebar-row", { hasText: "Network" }).click();
    await page.waitForSelector("#view .item:has-text('Google Drive')");
    // The connect toast would still be on screen; the dialog is the subject.
    await page.locator(".toast .flat-btn").click().catch(() => {});
    await page.evaluate(() =>
      window.__mockCalls.failNext(
        "list_dir",
        "Google Drive session expired: Server returned error response: invalid_grant: Token has been expired or revoked."
      )
    );
    await page.locator("#view .item", { hasText: "Google Drive" }).dblclick();
    await page.waitForFunction(
      () =>
        document.querySelector(".dialog-heading")?.textContent === "Sign In to Google Drive" ||
        [...document.querySelectorAll(".toast-title")].some((toast) => /session expired|Unable to access/.test(toast.textContent ?? "")),
      null,
      { timeout: 10_000 }
    );
  },
  // Connect to Arbesk from Online Accounts, with an email typed.
  "arbesk-connect": async (page) => {
    await openArbeskConnect(page);
    await page.fill("#arbesk-email", "me@example.com");
  },
  // An Arbesk collection in the list view, once the mock's account is connected.
  "arbesk-collection": async (page) => {
    await openMockArbeskCollection(page);
    await page.locator("#view .item", { hasText: "Box.gltf" }).click();
  },
  // The same collection in the grid view, once its assets' thumbnails are in.
  "arbesk-grid": async (page) => {
    await openMockArbeskCollection(page);
    await page.keyboard.press(`${MOD}+2`);
    await page.waitForTimeout(600);
  },
  // Network with the mock's Arbesk account connected.
  "network-arbesk": async (page) => {
    await openMockArbeskCollection(page);
    await page.locator(".sidebar-row", { hasText: "Network" }).click();
    await page.waitForSelector("#view .item:has-text('Arbesk')");
  },
  // Space on an Arbesk asset (--item, default Box.3mf).
  "arbesk-preview": async (page) => {
    await openMockArbeskCollection(page);
    await page.locator("#view .item").filter({ hasText: option("--item", "Box.3mf") }).click();
    await page.keyboard.press("Space");
    await page.waitForSelector("#preview-body > :not(.preview-loading)");
    await page.waitForTimeout(400);
  },
};

async function openArbeskConnect(page) {
  await page.locator("#btn-main-menu").click();
  await page.getByRole("menuitem", { name: "Online Accounts…", exact: true }).click();
  await page.locator(".dialog .action-row", { hasText: "Arbesk" }).click();
  await page.waitForSelector("#arbesk-email");
}

/** Connect the mock's Arbesk account, then open its Shapes collection as a list. */
async function openMockArbeskCollection(page) {
  await openArbeskConnect(page);
  await page.fill("#arbesk-email", "me@example.com");
  await page.getByRole("button", { name: "Open Browser", exact: true }).click();
  await page.waitForSelector("#view .item:has-text('Shapes')");
  await page.locator(".toast .flat-btn").click().catch(() => {});
  await page.locator("#view .item", { hasText: "Shapes" }).dblclick();
  await page.waitForSelector("#view .item:has-text('Box.3mf')");
  await page.keyboard.press(`${MOD}+1`);
  await page.waitForTimeout(300);
}

/** Connect the mock's Google account through Online Accounts and its dialogs. */
async function connectMockDrive(page) {
  await page.locator("#btn-main-menu").click();
  await page.getByRole("menuitem", { name: "Online Accounts…", exact: true }).click();
  await page.locator(".dialog .action-row", { hasText: "Google Drive" }).click();
  await page.fill(".client-json", JSON.stringify({ installed: { client_id: "1-a.apps.googleusercontent.com", client_secret: "s" } }));
  await page.getByRole("button", { name: "Add Client", exact: true }).click();
  await page.getByRole("button", { name: "Open Browser", exact: true }).click();
  await page.waitForSelector(".crumb.current:has-text('My Drive')");
}

/** Search the folder for --query and wait for the recursive results. */
async function search(page) {
  await page.keyboard.press(`${MOD}+f`);
  await page.keyboard.type(option("--query", "e"));
  await page.waitForFunction(() => !document.querySelector("#searchbar.busy"), null, { timeout: 10_000 });
}
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

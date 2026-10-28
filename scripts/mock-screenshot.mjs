#!/usr/bin/env node
// Screenshot the app in the mock preview, for a PR's before/after images.
// Needs `npm run dev:mock` running (port 1420) and Playwright installed
// (`npm install --no-save playwright && npx playwright install chromium`).
//
//   node scripts/mock-screenshot.mjs /Users/me/Pictures after.png
//   node scripts/mock-screenshot.mjs /Users/me/Pictures after-dark.png --dark
//
// Options: --dark, --url <base> (default http://localhost:1420),
// --size <width>x<height> (default 1100x720). Uses ?demo (no test-only
// folders) and a Mac user agent, so the app takes its macOS code paths.
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const option = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
};
const [path, out] = args.filter((a, i) => !a.startsWith("--") && !["--url", "--size"].includes(args[i - 1]));
if (!path || !out) {
  console.error("usage: node scripts/mock-screenshot.mjs <path in the mock> <out.png> [--dark] [--url <base>] [--size WxH]");
  process.exit(2);
}

let chromium;
try {
  ({ chromium } = await import("playwright"));
} catch {
  console.error("Playwright is missing: npm install --no-save playwright && npx playwright install chromium");
  process.exit(1);
}

const [width, height] = option("--size", "1100x720").split("x").map(Number);
const base = option("--url", "http://localhost:1420");
const browser = await chromium.launch();
try {
  const page = await browser.newPage({
    viewport: { width, height },
    deviceScaleFactor: 2,
    colorScheme: flag("--dark") ? "dark" : "light",
    userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15",
  });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${base}/?demo&path=${encodeURIComponent(path)}`);
  await page.waitForSelector("#view .item, #view .status-page", { timeout: 10_000 });
  // Let thumbnails and counts arrive (the mock answers in ~150 ms).
  await page.waitForTimeout(1000);
  await page.screenshot({ path: out });
  console.log(`Saved ${out}`);
  if (errors.length) {
    console.error(`Page errors:\n${errors.join("\n")}`);
    process.exitCode = 1;
  }
} finally {
  await browser.close();
}

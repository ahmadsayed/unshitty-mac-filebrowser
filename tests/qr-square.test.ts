// The QR code's box must stay square in every theme: a large corner radius
// (the macOS theme's capsule buttons use 999px) clips the finder patterns in
// the corners and the code no longer scans.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "vitest";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (path: string) => readFileSync(join(root, path), "utf8");

/** The value of `property` in the first rule for exactly `selector`. */
function declared(css: string, selector: string, property: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const rule = css.match(new RegExp(`(^|\\n)${escaped}\\s*\\{([^}]*)\\}`))?.[2] ?? "";
  return rule.match(new RegExp(`(^|[\\s;])${property}\\s*:\\s*([^;]+);`))?.[2].trim() ?? "";
}

/** Resolve `var(--token)` against a theme's tokens (light values come first). */
function resolve(value: string, theme: string): string {
  const token = value.match(/^var\((--[\w-]+)\)$/)?.[1];
  if (!token) return value;
  return theme.match(new RegExp(`${token}\\s*:\\s*([^;]+);`))?.[1].trim() ?? "";
}

test.each(["gnome", "macos"])("the QR code's corners are barely rounded in the %s theme", (name) => {
  const radius = resolve(declared(read("src/styles.css"), ".qr-share-code", "border-radius"), read(`src/themes/${name}.css`));
  expect(radius).toMatch(/^\d+px$/);
  // The code has a 4-module quiet zone (over 8 px at 240 px); keep the corners inside it.
  expect(parseInt(radius, 10)).toBeLessThanOrEqual(8);
});

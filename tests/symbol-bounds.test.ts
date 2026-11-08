// Symbolic icons stay inside their 16×16 box. An arc whose radius is too small
// to reach its end point is scaled up by the browser, so it bulges further
// than drawn: that pushed the cloud's right lobe out of the box, and Network's
// Google Drive icon was cut off on the right.
import { expect, test } from "vitest";
import { spriteMarkup } from "../src/icons";

type Point = [number, number];
type Command = { cmd: string; args: number[] };

const NUMBER = /^[\s,]*(-?(?:\d+\.?\d*|\.\d+)(?:e-?\d+)?)/i;
const FLAG = /^[\s,]*([01])/;

/** Reads numbers off the front of a command's text. */
class Reader {
  constructor(private rest: string) {}
  next(pattern: RegExp): number | null {
    const m = this.rest.match(pattern);
    if (!m) return null;
    this.rest = this.rest.slice(m[0].length);
    return Number(m[1]);
  }
}

/** One arc's seven numbers; its two flags may be written without separators. */
function arcArgs(reader: Reader): number[] | null {
  const rx = reader.next(NUMBER);
  if (rx === null) return null;
  const pattern = [NUMBER, NUMBER, FLAG, FLAG, NUMBER, NUMBER];
  return [rx, ...pattern.map((p) => reader.next(p) ?? NaN)];
}

/** The path's commands with their numbers. */
function commands(d: string): Command[] {
  const out: Command[] = [];
  for (const [, cmd, body] of d.matchAll(/([MmLlHhVvCcSsQqTtAaZz])([^MmLlHhVvCcSsQqTtAaZz]*)/g)) {
    const reader = new Reader(body);
    const args: number[] = [];
    const arc = cmd === "A" || cmd === "a";
    for (let more = arc ? arcArgs(reader) : [reader.next(NUMBER)]; more && more[0] !== null; more = arc ? arcArgs(reader) : [reader.next(NUMBER)]) {
      args.push(...(more as number[]));
    }
    out.push({ cmd, args });
  }
  return out;
}

const ARITY: Record<string, number> = { M: 2, L: 2, H: 1, V: 1, C: 6, S: 4, Q: 4, T: 2, A: 7, Z: 0 };

/** Where one segment ends, from where it starts. */
function segmentEnd(up: string, rel: boolean, a: number[], [x, y]: Point): Point {
  const [ox, oy] = rel ? [x, y] : [0, 0];
  if (up === "H") return [a[0] + ox, y];
  if (up === "V") return [x, a[0] + oy];
  return [a[a.length - 2] + ox, a[a.length - 1] + oy];
}

/** An arc too short to reach from `from` to `to` at its radius, described; null if it reaches. */
function shortArc(a: number[], from: Point, to: Point): string | null {
  const chord = Math.hypot(to[0] - from[0], to[1] - from[1]);
  const diameter = 2 * Math.max(a[0], a[1]);
  return chord > diameter + 0.02 ? `A${a.join(" ")} spans ${chord.toFixed(2)} > ${diameter}` : null;
}

/** Every end point the path reaches, and each arc too short for its end points. */
function walk(d: string): { points: Point[]; badArcs: string[] } {
  let at: Point = [0, 0];
  let start: Point = [0, 0];
  const points: Point[] = [];
  const badArcs: string[] = [];
  for (const { cmd, args } of commands(d)) {
    const up = cmd.toUpperCase();
    if (up === "Z") {
      at = start;
      continue;
    }
    const n = ARITY[up];
    for (let i = 0; i < args.length; i += n) {
      const a = args.slice(i, i + n);
      const next = segmentEnd(up, cmd !== up, a, at);
      const bad = up === "A" ? shortArc(a, at, next) : null;
      if (bad) badArcs.push(bad);
      if (up === "M" && i === 0) start = next;
      at = next;
      points.push(at);
    }
  }
  return { points, badArcs };
}

const outside = ([x, y]: Point) => x < 0 || x > 16 || y < 0 || y > 16;

/** Every problem with one symbol's paths. */
function symbolProblems(symbol: Element): string[] {
  return [...symbol.querySelectorAll("path")].flatMap((path) => {
    const { points, badArcs } = walk(path.getAttribute("d") ?? "");
    const stray = points.filter(outside).map(([x, y]) => `point ${x},${y} is outside 0–16`);
    return [...badArcs, ...stray].map((problem) => `${symbol.id}: ${problem}`);
  });
}

test.each(["gnome", "macos"] as const)("every %s symbolic glyph's paths stay inside the box", (theme) => {
  const doc = new DOMParser().parseFromString(`<svg xmlns="http://www.w3.org/2000/svg">${spriteMarkup(theme)}</svg>`, "image/svg+xml");
  expect([...doc.querySelectorAll('symbol[id^="sym-"]')].flatMap(symbolProblems)).toEqual([]);
});

test("the path walker catches an arc too short for its end points, as the old cloud had", () => {
  expect(walk("M12.45 5.05A3.5 3.5 0 0 1 11.75 13").badArcs).toHaveLength(1);
  expect(walk("M4.5 12.75A3 3 0 0 1 4.03 6.79").badArcs).toEqual([]);
  expect(walk("M1 1h3v3H1z").points).toEqual([[1, 1], [4, 1], [4, 4], [1, 4]]);
  expect(walk("M20 1").points.filter(outside)).toEqual([[20, 1]]);
});

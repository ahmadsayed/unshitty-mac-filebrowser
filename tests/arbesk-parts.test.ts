// An Arbesk scene's parts fetched a few at a time (performance plan, phase
// 3: src/arbesk-parts.ts) give what fetching them one after another gives:
// the same parts taken, in the scene's order, the same "too large", and
// what is held and on its way never passes the budget.
import { expect, test } from "vitest";
import { fetchParts, PARTS_AT_ONCE } from "../src/arbesk-parts";

const TOO_LARGE = "This file is too large to preview.";

/** A scene of parts, each its size, or failing as the gateway would. */
type Part = { size: number; fails?: boolean; badParse?: boolean };

/** The loop the window replaced: one part after another, each asked with
 *  what is left. */
async function inTurn(parts: Part[], budget: number): Promise<{ taken: number[]; tooLarge: boolean }> {
  const taken: number[] = [];
  let left = budget;
  let tooLarge = false;
  for (const [index, part] of parts.entries()) {
    if (left <= 0) {
      tooLarge = true;
      break;
    }
    try {
      if (part.fails) throw "Arbesk's storage could not be reached: Bad gateway";
      if (part.size > left) throw TOO_LARGE;
      left -= part.size;
      if (part.badParse) throw new Error("Unexpected end of data");
      taken.push(index);
    } catch (err) {
      tooLarge ||= /too large/i.test(String(err));
    }
  }
  return { taken, tooLarge };
}

/** A seeded random number in [0, 1). */
function seeded(seed: number): () => number {
  let state = seed;
  return () => {
    state = (state * 1_103_515_245 + 12_345) % 2 ** 31;
    return state / 2 ** 31;
  };
}

/** `parts` through the window, each answer late by a random few ms (so
 *  out of order), watching what is held and on its way. */
async function together(parts: Part[], budget: number, random: () => number) {
  const taken: number[] = [];
  let inFlight = 0;
  let peak = 0;
  let worst = 0;
  let parsing = 0;
  /** Each call's limit while it runs, each landed part's bytes until taken. */
  const onItsWay = new Map<number, number>();
  const landed = new Map<number, number>();
  const asked = new Set<number>();
  let takenBytes = 0;
  const held = () => takenBytes + [...onItsWay.values(), ...landed.values()].reduce((a, b) => a + b, 0);
  const result = await fetchParts({
    count: parts.length,
    budget,
    fetch: async (index, limit) => {
      inFlight += 1;
      peak = Math.max(peak, inFlight);
      // A part asked again is asked alone: the parts after it were let go.
      if (asked.has(index)) [...landed.keys()].filter((later) => later > index).forEach((later) => landed.delete(later));
      asked.add(index);
      landed.delete(index);
      onItsWay.set(index, limit);
      worst = Math.max(worst, held());
      await new Promise((resolve) => setTimeout(resolve, Math.floor(random() * 4)));
      inFlight -= 1;
      onItsWay.delete(index);
      const part = parts[index];
      if (part.fails) throw "Arbesk's storage could not be reached: Bad gateway";
      if (part.size > limit) throw TOO_LARGE;
      landed.set(index, part.size);
      return new ArrayBuffer(part.size);
    },
    take: async (index, bytes) => {
      expect(parsing).toBe(0);
      parsing += 1;
      landed.delete(index);
      takenBytes += bytes.byteLength;
      await new Promise((resolve) => setTimeout(resolve, Math.floor(random() * 2)));
      parsing -= 1;
      if (parts[index].badParse) throw new Error("Unexpected end of data");
      taken.push(index);
    },
    stale: () => false,
  });
  return { ...result, taken, peak, worst };
}

test("parts answering out of order are taken in the scene's order, as fetching them in turn takes them", async () => {
  const random = seeded(7);
  for (let round = 0; round < 150; round++) {
    const budget = 50 + Math.floor(random() * 200);
    const parts: Part[] = Array.from({ length: 1 + Math.floor(random() * 9) }, () => ({
      size: Math.floor(random() * 90),
      fails: random() < 0.1,
      badParse: random() < 0.05,
    }));
    const got = await together(parts, budget, random);
    const want = await inTurn(parts, budget);
    expect({ taken: got.taken, tooLarge: got.tooLarge }, JSON.stringify({ budget, parts })).toEqual(want);
    expect(got.worst, JSON.stringify({ budget, parts })).toBeLessThanOrEqual(budget);
    expect(got.peak).toBeLessThanOrEqual(PARTS_AT_ONCE);
  }
});

test("calls go together, never more than PARTS_AT_ONCE", async () => {
  const parts = Array.from({ length: 10 }, () => ({ size: 1 }));
  const got = await together(parts, 1000, seeded(3));
  expect(got.peak).toBe(PARTS_AT_ONCE);
  expect(got.taken).toEqual([...parts.keys()]);
});

test("a part too large for its share is asked again alone with all that is left", async () => {
  const limits: Array<[number, number]> = [];
  const sizes = [10, 80, 10, 10];
  const taken: number[] = [];
  const result = await fetchParts({
    count: sizes.length,
    budget: 100,
    fetch: async (index, limit) => {
      limits.push([index, limit]);
      if (sizes[index] > limit) throw TOO_LARGE;
      return new ArrayBuffer(sizes[index]);
    },
    take: async (index) => void taken.push(index),
    stale: () => false,
  });
  // Shared four ways, then the 80 alone with the 90 the first left.
  expect(limits.slice(0, 4)).toEqual([[0, 25], [1, 25], [2, 25], [3, 25]]);
  expect(limits).toContainEqual([1, 90]);
  expect(taken).toEqual([0, 1, 2]);
  // Fetching in turn runs out too: 10 + 80 + 10 leaves nothing for the last.
  expect(result.tooLarge).toBe(true);
  expect(await inTurn(sizes.map((size) => ({ size })), 100)).toEqual({ taken: [0, 1, 2], tooLarge: true });
});

test("a stale preview starts no more calls and takes nothing more", async () => {
  let stale = false;
  const asked: number[] = [];
  const taken: number[] = [];
  const result = await fetchParts({
    count: 10,
    budget: 1000,
    fetch: async (index) => {
      asked.push(index);
      // The preview is closed as the first call starts.
      stale = true;
      return new ArrayBuffer(1);
    },
    take: async (index) => void taken.push(index),
    stale: () => stale,
  });
  expect(asked).toEqual([0]);
  expect(taken).toEqual([]);
  expect(result.tooLarge).toBe(false);
});

test("one failed part leaves the others", async () => {
  const taken: number[] = [];
  await fetchParts({
    count: 5,
    budget: 1000,
    fetch: async (index) => {
      if (index === 2) throw "Arbesk's storage could not be reached: Bad gateway";
      return new ArrayBuffer(1);
    },
    take: async (index) => void taken.push(index),
    stale: () => false,
  });
  expect(taken).toEqual([0, 1, 3, 4]);
});

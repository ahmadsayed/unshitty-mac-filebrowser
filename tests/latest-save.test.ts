// LatestSave (src/latest-save.ts): one write of a file at a time, each with
// the state as it is when it starts, so the last write is the latest state
// (#172).
import { expect, test } from "vitest";
import { LatestSave } from "../src/latest-save";

/** A file whose writes wait until the test ends them, in any order. */
function fakeFile() {
  const writes: { value: string[]; end: () => void; fail: (err: unknown) => void }[] = [];
  const write = (value: string[]) =>
    new Promise<void>((resolve, reject) => writes.push({ value, end: resolve, fail: reject }));
  return { writes, write };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

test("a save with nothing running writes the state at once", async () => {
  let state = ["a"];
  const file = fakeFile();
  const saves = new LatestSave(() => [...state], file.write, () => {});
  const saved = saves.save();
  expect(file.writes.map((w) => w.value)).toEqual([["a"]]);
  state = ["b"];
  file.writes[0].end();
  expect(await saved).toBe(true);
  await tick();
  expect(file.writes).toHaveLength(1);
});

test("saves asked while one writes share one write after it, with the latest state", async () => {
  let state = ["a"];
  const file = fakeFile();
  const saves = new LatestSave(() => [...state], file.write, () => {});
  const order: string[] = [];
  const first = saves.save().then(() => order.push("first"));
  state = ["a", "b"];
  const second = saves.save().then(() => order.push("second"));
  state = ["a", "b", "c"];
  const third = saves.save().then(() => order.push("third"));
  // One write at a time: the others wait.
  expect(file.writes).toHaveLength(1);

  file.writes[0].end();
  await first;
  await tick();
  expect(order).toEqual(["first"]);
  expect(file.writes.map((w) => w.value)).toEqual([["a"], ["a", "b", "c"]]);

  file.writes[1].end();
  await Promise.all([second, third]);
  expect(order).toEqual(["first", "second", "third"]);
  await tick();
  expect(file.writes).toHaveLength(2);
});

test("every save a write carried answers true once it is written", async () => {
  const file = fakeFile();
  const saves = new LatestSave(() => ["a"], file.write, () => {});
  const first = saves.save();
  const second = saves.save();
  const third = saves.save();
  file.writes[0].end();
  await tick();
  file.writes[1].end();
  expect(await Promise.all([first, second, third])).toEqual([true, true, true]);
});

test("a save asked once the queued write started waits for one more", async () => {
  let state = ["a"];
  const file = fakeFile();
  const saves = new LatestSave(() => [...state], file.write, () => {});
  void saves.save();
  state = ["b"];
  void saves.save();
  file.writes[0].end();
  await tick();
  expect(file.writes.map((w) => w.value)).toEqual([["a"], ["b"]]);

  state = ["c"];
  let done = false;
  const late = saves.save().then(() => (done = true));
  file.writes[1].end();
  await tick();
  expect(done).toBe(false);
  expect(file.writes.map((w) => w.value)).toEqual([["a"], ["b"], ["c"]]);
  file.writes[2].end();
  await late;
});

test("a failed write is reported once and answers false to each save it carried; the next save writes again", async () => {
  let state = ["a"];
  const file = fakeFile();
  const failures: unknown[] = [];
  const saves = new LatestSave(() => [...state], file.write, (err) => failures.push(err));
  const first = saves.save();
  state = ["b"];
  const second = saves.save();
  const third = saves.save();
  file.writes[0].fail("disk full");
  expect(await first).toBe(false);
  expect(failures).toEqual(["disk full"]);
  await tick();
  file.writes[1].fail("still full");
  expect(await Promise.all([second, third])).toEqual([false, false]);
  expect(failures).toEqual(["disk full", "still full"]);

  const fourth = saves.save();
  expect(file.writes.map((w) => w.value)).toEqual([["a"], ["b"], ["b"]]);
  file.writes[2].end();
  expect(await fourth).toBe(true);
  expect(failures).toHaveLength(2);
});

test("a write that throws before it starts is reported like a failed one", async () => {
  const failures: unknown[] = [];
  const saves = new LatestSave(
    () => ["a"],
    () => {
      throw "no file";
    },
    (err) => failures.push(err),
  );
  expect(await saves.save()).toBe(false);
  expect(failures).toEqual(["no file"]);
});

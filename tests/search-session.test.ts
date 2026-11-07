// SearchSession (src/search.ts) without the app: a fake walk the test
// answers, fake timers for the pause, and the changes it reports.
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { Entry, SearchSummary } from "../src/api";
import { SEARCH_DELAY_MS, SEARCH_LIMIT, SearchSession, type SearchChange } from "../src/search";

const entry = (path: string): Entry =>
  ({ path, name: path.split("/").pop()!, is_dir: false, size: 0, modified: 0, is_hidden: false, is_symlink: false });

interface Walk {
  at: string;
  query: string;
  limit: number;
  send(batch: Entry[]): void;
  end(summary?: SearchSummary): Promise<void>;
  fail(error: unknown): Promise<void>;
}

let walks: Walk[];
let cancels: number;
let changes: Array<[SearchChange, unknown?]>;
let session: SearchSession;

/** Let promise callbacks run. */
const flush = () => new Promise<void>((resolve) => queueMicrotask(() => queueMicrotask(() => queueMicrotask(resolve))));

beforeEach(() => {
  vi.useFakeTimers();
  walks = [];
  cancels = 0;
  changes = [];
  session = new SearchSession(
    (at, query, limit, onFound) =>
      new Promise<SearchSummary>((resolve, reject) => {
        walks.push({
          at, query, limit,
          send: onFound,
          end: async (summary = { stopped: null, scanned: 1 }) => (resolve(summary), flush()),
          fail: async (error) => (reject(error), flush()),
        });
      }),
    async () => {
      cancels += 1;
    },
    (change, error) => changes.push(error === undefined ? [change] : [change, error]),
  );
});

afterEach(() => {
  vi.useRealTimers();
});

const kinds = () => changes.map(([change]) => change);

test("closed until opened; opening changes nothing until there is text", () => {
  expect(session.isOpen).toBe(false);
  expect(session.saved).toBeNull();
  expect(session.text).toBe("");
  session.setText("ignored while closed");
  session.rerun();
  session.close();
  expect(changes).toEqual([]);

  session.open("/home/me", true);
  expect(session.isOpen).toBe(true);
  expect(session.saved).toBe("");
  expect(changes).toEqual([]);
  expect(session.busy).toBe(false);
});

test("a recursive place walks after the pause, with the trimmed text and the limit", () => {
  session.open("/home/me", true);
  session.setText("  notes ");
  expect(kinds()).toEqual(["restarted"]);
  expect(session.busy).toBe(true);
  expect(session.text).toBe("  notes ");
  expect(session.saved).toBe("  notes ");
  vi.advanceTimersByTime(SEARCH_DELAY_MS - 1);
  expect(walks).toHaveLength(0);
  vi.advanceTimersByTime(1);
  expect(walks.map((w) => [w.at, w.query, w.limit])).toEqual([["/home/me", "notes", SEARCH_LIMIT]]);
});

test("typing again within the pause starts only the last text's walk", () => {
  session.open("/home/me", true);
  session.setText("no");
  vi.advanceTimersByTime(100);
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  expect(walks.map((w) => w.query)).toEqual(["notes"]);
  expect(cancels).toBe(0);
});

test("a place that doesn't walk, or a blank text, never walks", () => {
  session.open("pseudo:recent", false);
  session.setText("notes");
  expect(session.busy).toBe(false);
  session.open("/home/me", true);
  session.setText("   ");
  expect(session.busy).toBe(false);
  vi.advanceTimersByTime(SEARCH_DELAY_MS * 2);
  expect(walks).toHaveLength(0);
  expect(kinds()).toEqual(["restarted", "restarted"]);
});

test("batches grow the results, each time in a new array; the end keeps them and says how it stopped", async () => {
  session.open("/home/me", true);
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  expect(session.results).toBeNull();
  const [walk] = walks;
  walk.send([entry("/home/me/a/notes.txt")]);
  const first = session.results;
  walk.send([entry("/home/me/b/notes.md")]);
  expect(session.results).not.toBe(first);
  expect(session.results?.map((e) => e.name)).toEqual(["notes.txt", "notes.md"]);
  expect(first?.map((e) => e.name)).toEqual(["notes.txt"]);
  const grown = session.results;
  await walk.end({ stopped: "results", scanned: 40 });
  expect(kinds()).toEqual(["restarted", "grew", "grew", "ended"]);
  expect(session.results).toBe(grown);
  expect(session.busy).toBe(false);
  expect(session.stop).toEqual({ stopped: "results", scanned: 40 });
});

test("a walk that ends without a batch leaves empty results, not none", async () => {
  session.open("/home/me", true);
  session.setText("zzz");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  await walks[0].end();
  expect(session.results).toEqual([]);
  expect(kinds()).toEqual(["restarted", "ended"]);
});

test("a failed walk empties the results and passes the error on", async () => {
  session.open("/home/me", true);
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  walks[0].send([entry("/home/me/a/notes.txt")]);
  await walks[0].fail("Permission denied");
  expect(changes.at(-1)).toEqual(["failed", "Permission denied"]);
  expect(session.results).toEqual([]);
  expect(session.busy).toBe(false);
  expect(session.stop).toBeNull();
});

test("a restart forgets the results and the stop, and drops the old walk's answers", async () => {
  session.open("/home/me", true);
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  const [old] = walks;
  old.send([entry("/home/me/a/notes.txt")]);
  session.setText("old");
  expect(session.results).toBeNull();
  expect(session.busy).toBe(true);
  // The next walk replaces it in the backend: nothing to cancel here.
  expect(cancels).toBe(0);
  old.send([entry("/home/me/b/notes.md")]);
  await old.end({ stopped: "scanned", scanned: 100 });
  await old.fail("late");
  expect(kinds()).toEqual(["restarted", "grew", "restarted"]);
  expect(session.results).toBeNull();
  expect(session.stop).toBeNull();
  expect(session.busy).toBe(true);
});

test("an ended search's stop goes at the next restart, and rerun walks again", async () => {
  session.open("/home/me", true);
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  await walks[0].end({ stopped: "results", scanned: 5 });
  session.rerun();
  expect(session.stop).toBeNull();
  expect(session.results).toBeNull();
  expect(session.text).toBe("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  expect(walks.map((w) => w.query)).toEqual(["notes", "notes"]);
});

test("a text that no longer walks cancels the running walk", () => {
  session.open("/home/me", true);
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  session.setText("");
  expect(cancels).toBe(1);
  expect(session.busy).toBe(false);
  session.setText(" ");
  expect(cancels).toBe(1);
});

test("closing cancels a walk, even one still waiting for the pause, which then never starts", async () => {
  session.open("/home/me", true);
  session.setText("notes");
  session.close();
  expect(cancels).toBe(1);
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  expect(walks).toHaveLength(0);
  expect(session.isOpen).toBe(false);
  expect(session.saved).toBeNull();
  expect(session.text).toBe("");
  expect(session.busy).toBe(false);
  expect(kinds()).toEqual(["restarted"]);
});

test("closing drops the walk's late answers; an ended search has nothing to cancel", async () => {
  session.open("/home/me", true);
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  const [walk] = walks;
  session.close();
  walk.send([entry("/home/me/a/notes.txt")]);
  await walk.end();
  expect(session.results).toBeNull();
  expect(kinds()).toEqual(["restarted"]);

  session.open("/home/me", true);
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  await walks[1].end();
  session.close();
  expect(cancels).toBe(1);
});

test("opening again closes first: the old walk is cancelled and its answers dropped", async () => {
  session.open("/home/me", true);
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  session.open("/home/me/Work", true);
  expect(cancels).toBe(1);
  expect(session.text).toBe("");
  walks[0].send([entry("/home/me/a/notes.txt")]);
  expect(session.results).toBeNull();
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  expect(walks.map((w) => w.at)).toEqual(["/home/me", "/home/me/Work"]);
});

test("moving to another place drops the old place's answers and walks the new one", async () => {
  session.open("/home/me", true);
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  const [old] = walks;
  old.send([entry("/home/me/a/notes.txt")]);
  session.moveTo("/home/me/Work", true);
  expect(session.text).toBe("notes");
  expect(session.results).toBeNull();
  expect(session.busy).toBe(true);
  old.send([entry("/home/me/b/notes.md")]);
  await old.end();
  expect(session.results).toBeNull();
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  expect(walks.map((w) => w.at)).toEqual(["/home/me", "/home/me/Work"]);
  expect(kinds()).toEqual(["restarted", "grew", "restarted"]);
  // The text changing afterwards walks the new place too.
  session.setText("notes-old");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  expect(walks.at(-1)?.at).toBe("/home/me/Work");
});

test("moving to a place that doesn't walk cancels the walk; moving back walks again", () => {
  session.open("/home/me", true);
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  walks[0].send([entry("/home/me/a/notes.txt")]);
  session.moveTo("pseudo:recent", false);
  expect(cancels).toBe(1);
  expect(session.busy).toBe(false);
  expect(session.results).toBeNull();
  walks[0].send([entry("/home/me/b/notes.md")]);
  expect(session.results).toBeNull();
  session.setText("note");
  expect(session.busy).toBe(false);
  session.moveTo("/home/me/Work", true);
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  expect(walks.map((w) => [w.at, w.query])).toEqual([["/home/me", "notes"], ["/home/me/Work", "note"]]);
});

test("moving with no text, or while closed, reports nothing", () => {
  session.moveTo("/home/me/Work", true);
  expect(session.isOpen).toBe(false);
  session.open("pseudo:recent", false);
  session.moveTo("/home/me/Work", true);
  expect(changes).toEqual([]);
  session.setText("notes");
  vi.advanceTimersByTime(SEARCH_DELAY_MS);
  expect(walks.map((w) => w.at)).toEqual(["/home/me/Work"]);
});

test("a failing cancel is ignored", async () => {
  const quiet = new SearchSession(
    () => new Promise<SearchSummary>(() => {}),
    () => Promise.reject(new Error("no window")),
    () => {},
  );
  quiet.open("/home/me", true);
  quiet.setText("notes");
  quiet.close();
  await flush();
  expect(quiet.isOpen).toBe(false);
});

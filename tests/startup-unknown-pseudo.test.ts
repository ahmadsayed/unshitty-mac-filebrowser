// A window opened with ?path= naming a pseudo location the app doesn't
// have: it opens Home, like any other path it doesn't know.
import { beforeAll, expect, test } from "vitest";
import { $$, boot, title } from "./app";

beforeAll(async () => {
  window.history.replaceState(null, "", `/?path=${encodeURIComponent("pseudo:bogus")}`);
  await boot();
});

test("?path= with an unknown pseudo location opens Home", () => {
  expect(title()).toBe("Home");
  expect($$("#crumbs .crumb").map((c) => c.dataset.path)).toEqual(["/Users/me"]);
});

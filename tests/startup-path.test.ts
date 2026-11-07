// A window opened with ?path= (New Window from an SSH folder). The window
// opens only the kinds of path it knows; anything else opens Home.
import { beforeAll, expect, test } from "vitest";
import { $$, boot, title } from "./app";

beforeAll(async () => {
  window.history.replaceState(null, "", `/?path=${encodeURIComponent("sftp://user@host/")}`);
  await boot();
});

test("?path= with an SSH location opens Home", () => {
  // today's behaviour, fixed in a follow-up: the init whitelist omits
  // sftp://, so New Window from an SSH folder opens Home.
  expect(title()).toBe("Home");
  expect($$("#crumbs .crumb").map((c) => c.dataset.path)).toEqual(["/Users/me"]);
});

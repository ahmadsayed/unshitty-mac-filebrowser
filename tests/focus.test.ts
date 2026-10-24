// Returning from another app must re-list the folder: Word saves a file while
// the browser is in the background, then the user comes back to it.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { boot, goTo, names, settle } from "./app";

beforeAll(async () => {
  await boot();
});

test("the current folder is re-listed when the window regains focus", async () => {
  await goTo("/Users/me/Documents");
  const path = "/Users/me/Documents/Focused";
  await invoke("make_dir", { path });
  expect(names()).not.toContain("Focused");

  window.dispatchEvent(new Event("focus"));
  await settle(30);
  expect(names()).toContain("Focused");
});

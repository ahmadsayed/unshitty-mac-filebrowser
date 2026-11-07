// A window opened with ?path= (New Window from an SSH folder). The window
// opens only the kinds of path it knows; anything else opens Home.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $$, boot, names, title } from "./app";

beforeAll(async () => {
  // Connections belong to the app, not a window: the new window finds the
  // server already connected.
  await invoke("sftp_connect", { url: "user@host" });
  window.history.replaceState(null, "", `/?path=${encodeURIComponent("sftp://user@host/home/user/Documents")}`);
  await boot();
});

test("?path= with an SSH location opens that folder on the server", () => {
  expect(title()).toBe("Documents");
  expect($$("#crumbs .crumb").map((c) => c.dataset.path)).toEqual([
    "sftp://user@host/",
    "sftp://user@host/home",
    "sftp://user@host/home/user",
    "sftp://user@host/home/user/Documents",
  ]);
  expect(names()).toEqual(["hello.txt"]);
});

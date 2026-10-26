// A folder can vanish while its view is open: a cancelled copy deletes the
// folder it had been filling in. Reloading must not strand the window on
// "Unable to Access Location".
import { expect, test } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { boot, goTo, key, names, settle, title } from "./app";

const DOCS = "/Users/me/Documents";

test("reloading a folder that was removed goes to its parent", async () => {
  await boot();
  await goTo(`${DOCS}/Projects`);
  expect(title()).toBe("Projects");

  await invoke("trash_entries", { paths: [`${DOCS}/Projects`] });
  await key("F5");
  await settle(20);

  expect(document.body.textContent).not.toContain("Unable to Access Location");
  expect(title()).toBe("Documents");
  expect(names()).toContain("Notes.md");
});

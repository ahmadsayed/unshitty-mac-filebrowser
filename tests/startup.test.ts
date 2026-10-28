// Launch: the first folder appears without waiting for the network. Checking
// received shares can take a connect timeout per offline sharer, so it must
// not hold up the local view; share emblems arrive once it is done.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { boot, item, names, settle } from "./app";

const mockShare = () => (globalThis as unknown as { __mockShare: { holdReceived(hold: boolean): void } }).__mockShare;
const shared = (name: string) => !!item(name).querySelector(".icon .emblem");

beforeAll(async () => {
  await invoke("share_folder", { path: "/Users/me/Documents" });
  mockShare().holdReceived(true);
  await boot();
});

test("Home appears while received shares are still being checked", async () => {
  expect(names()).toContain("Documents");
  expect(names()).toContain("Desktop");
});

test("share emblems appear once the shares have been checked", async () => {
  expect(shared("Documents")).toBe(false);
  mockShare().holdReceived(false);
  await settle();
  expect(shared("Documents")).toBe(true);
  expect(shared("Desktop")).toBe(false);
});

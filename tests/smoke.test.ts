import { expect, test } from "vitest";
import { boot, names, title } from "./app";

test("the app boots in the home folder", async () => {
  await boot();
  expect(title()).toBe("Home");
  expect(names().length).toBeGreaterThan(5);
});

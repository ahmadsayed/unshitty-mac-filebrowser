// The window buttons Linux gets in place of a title bar, placed by GNOME's
// button-layout setting (the mock reads it from __mockButtonLayout).
import { beforeAll, expect, test } from "vitest";
import { $, $$, boot, click, settle } from "./app";

type MockWindow = { maximized: boolean; actions: string[] };
const mockWindow = () => (globalThis as unknown as { __mockWindow: MockWindow }).__mockWindow;

beforeAll(async () => {
  (globalThis as { __mockButtonLayout?: string }).__mockButtonLayout = "close:minimize,maximize";
  await boot();
});

test("buttons before the colon go in the sidebar header, the rest in the header bar", () => {
  const labels = (sel: string) => $$(`${sel} .window-control`).map((btn) => btn.title);
  expect(labels(".sidebar-header")).toEqual(["Close"]);
  expect(labels(".headerbar")).toEqual(["Minimize", "Maximize"]);
  // At the far ends, like GNOME Files.
  expect($(".sidebar-header").firstElementChild!.classList).toContain("window-controls");
  expect($(".headerbar").lastElementChild!.classList).toContain("window-controls");
});

test("Maximize becomes Restore while maximized; the buttons act on the window", async () => {
  await click($(".window-maximize"));
  await settle();
  expect($(".window-maximize").title).toBe("Restore");
  await click($(".window-maximize"));
  await settle();
  expect($(".window-maximize").title).toBe("Maximize");
  await click($(".window-minimize"));
  await click($(".window-close"));
  await settle();
  expect(mockWindow().actions).toEqual(["maximize", "maximize", "minimize", "close"]);
});

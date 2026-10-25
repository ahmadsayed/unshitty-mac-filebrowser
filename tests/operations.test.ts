// The operations model: titles, detail text and progress fractions shown in
// the header bar's operations popover (spec 2026-10-25-operation-progress).
import { afterEach, expect, test, vi } from "vitest";
import {
  endOperation, finishItem, initOperationsButton, opDetail, opFraction, opTitle, resetOperations,
  startOperation, updateOperation, type Operation,
} from "../src/operations";

function op(fields: Partial<Operation>): Operation {
  return {
    id: "op1",
    title: "Copying “a.txt”",
    state: "working",
    totalBytes: null,
    totalItems: 0,
    doneBytes: 0,
    doneItems: 0,
    current: null,
    startedAt: 0,
    speed: null,
    cancel: () => {},
    ...fields,
  };
}

test("titles use GNOME's wording", () => {
  expect(opTitle("copy", ["a.txt"])).toBe("Copying “a.txt”");
  expect(opTitle("copy", ["a", "b", "c"], "Documents")).toBe("Copying 3 files to “Documents”");
  expect(opTitle("move", ["a.txt"])).toBe("Moving “a.txt”");
  expect(opTitle("move", ["a", "b", "c"], "Documents")).toBe("Moving 3 files to “Documents”");
  expect(opTitle("trash", ["a.txt"])).toBe("Moving “a.txt” to the Trash");
  expect(opTitle("trash", ["a", "b", "c"])).toBe("Moving 3 files to the Trash");
  expect(opTitle("delete", ["a.txt"])).toBe("Deleting “a.txt”");
  expect(opTitle("delete", ["a", "b", "c"])).toBe("Deleting 3 files");
  expect(opTitle("empty", [])).toBe("Emptying the Trash");
  expect(opTitle("open", ["a.txt"])).toBe("Opening “a.txt”");
});

test("the detail line names waiting, preparing and cancelling", () => {
  expect(opDetail(op({ state: "waiting" }), 0)).toBe("Waiting…");
  expect(opDetail(op({ state: "preparing" }), 0)).toBe("Preparing…");
  expect(opDetail(op({ state: "cancelling" }), 0)).toBe("Cancelling…");
});

test("byte progress shows sizes, and time left only after 2 s", () => {
  const running = op({ totalBytes: 80.1e6, doneBytes: 12.4e6 });
  expect(opDetail(running, 1000)).toBe("12.4 MB of 80.1 MB");
  expect(opDetail(running, 3000)).toMatch(/^12\.4 MB of 80\.1 MB — .+ left$/);
});

test("the running item's bytes count towards the total", () => {
  const running = op({
    totalBytes: 10e6,
    doneBytes: 2e6,
    current: { bytes: 3e6, items: 0, total: null, phase: "working", done: false },
  });
  expect(opDetail(running, 1000)).toBe("5.0 MB of 10.0 MB");
  expect(opFraction(running)).toBeCloseTo(0.5);
});

test("time left reads like GNOME Files", () => {
  // 1 MB done per second, 59 MB to go.
  expect(opDetail(op({ totalBytes: 64e6, doneBytes: 5e6 }), 5000)).toBe("5.0 MB of 64.0 MB — 59 seconds left");
  // 1 MB per second, 120 MB to go.
  expect(opDetail(op({ totalBytes: 130e6, doneBytes: 10e6 }), 10_000)).toBe("10.0 MB of 130.0 MB — 2 minutes left");
});

test("item progress counts files when there is no byte total", () => {
  expect(opDetail(op({ totalItems: 10, doneItems: 3 }), 5000)).toBe("3 of 10 files");
});

test("fractions use bytes when known, else items", () => {
  expect(opFraction(op({ totalBytes: 100, doneBytes: 25 }))).toBeCloseTo(0.25);
  expect(opFraction(op({ totalItems: 4, doneItems: 1 }))).toBeCloseTo(0.25);
  expect(opFraction(op({}))).toBe(0);
});

test("an empty file copy reaches 100%", () => {
  expect(opFraction(op({ totalBytes: 0, totalItems: 1, doneItems: 1 }))).toBe(1);
});

// ---- the button and popover

function mountButton(): HTMLButtonElement {
  document.body.replaceChildren();
  const button = document.createElement("button");
  button.hidden = true;
  document.body.appendChild(button);
  initOperationsButton(button);
  return button;
}

const rows = () => [...document.querySelectorAll(".operation-row")];

afterEach(() => {
  resetOperations();
  vi.useRealTimers();
});

test("the button appears with a pie while an operation runs and opens the popover once", () => {
  vi.useFakeTimers();
  const button = mountButton();
  const op = startOperation("Copying “a.txt”", { totalItems: 1, onCancel: () => {} });
  expect(button.hidden).toBe(false);
  expect(button.querySelector(".op-pie")).not.toBeNull();
  vi.advanceTimersByTime(1000);
  expect(rows()).toHaveLength(1);
  expect(rows()[0].querySelector(".operation-title")!.textContent).toBe("Copying “a.txt”");
  expect(rows()[0].querySelector(".operation-detail")!.textContent).toBe("Preparing…");
  updateOperation(op, { bytes: 0, items: 0, total: null, phase: "working", done: false });
  expect(rows()[0].querySelector(".operation-detail")!.textContent).toBe("0 of 1 files");
});

test("closing the popover keeps the operation, and reopening shows its progress", () => {
  vi.useFakeTimers();
  const button = mountButton();
  const op = startOperation("Copying 2 files to “Documents”", { totalItems: 2, onCancel: () => {} });
  vi.advanceTimersByTime(1000);
  expect(rows()).toHaveLength(1);
  button.click();
  expect(rows()).toHaveLength(0);
  finishItem(op);
  button.click();
  expect(rows()[0].querySelector(".operation-detail")!.textContent).toBe("1 of 2 files");
});

test("Cancel marks the row cancelling and calls back once", () => {
  vi.useFakeTimers();
  mountButton();
  const onCancel = vi.fn();
  startOperation("Copying “a.txt”", { totalItems: 1, onCancel });
  vi.advanceTimersByTime(1000);
  const cancel = rows()[0].querySelector<HTMLButtonElement>(".operation-cancel")!;
  cancel.click();
  cancel.click();
  expect(onCancel).toHaveBeenCalledTimes(1);
  expect(rows()[0].querySelector(".operation-detail")!.textContent).toBe("Cancelling…");
});

test("the button shows a checkmark for 3 s after the last operation, then hides", () => {
  vi.useFakeTimers();
  const button = mountButton();
  const op = startOperation("Copying “a.txt”", { totalItems: 1, onCancel: () => {} });
  endOperation(op);
  expect(button.hidden).toBe(false);
  expect(button.querySelector(".op-pie")).toBeNull();
  vi.advanceTimersByTime(3000);
  expect(button.hidden).toBe(true);
});

test("a waiting operation says so", () => {
  vi.useFakeTimers();
  mountButton();
  startOperation("Copying “b.txt”", { totalItems: 1, onCancel: () => {}, waiting: true });
  vi.advanceTimersByTime(1000);
  expect(rows()[0].querySelector(".operation-detail")!.textContent).toBe("Waiting…");
});

test("a progress update keeps the row's elements, so a click in progress isn't lost", () => {
  vi.useFakeTimers();
  mountButton();
  const op = startOperation("Copying “a.txt”", { totalBytes: 100, totalItems: 1, onCancel: () => {} });
  vi.advanceTimersByTime(1000);
  const cancel = rows()[0].querySelector(".operation-cancel");
  updateOperation(op, { bytes: 40, items: 0, total: null, phase: "working", done: false });
  expect(rows()[0].querySelector(".operation-cancel")).toBe(cancel);
  expect(rows()[0].querySelector(".operation-detail")!.textContent).toBe("40 bytes of 100 bytes");
});


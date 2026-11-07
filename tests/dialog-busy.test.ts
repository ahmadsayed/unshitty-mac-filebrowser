// openDialog's busy mode: one action in flight, what it disables and puts
// back, its status line and its failures. The dialogs that use it are pinned
// in dialog-in-flight.test.ts.
import { afterEach, expect, test, vi } from "vitest";
import { cleanError, openDialog, type DialogBusy } from "../src/ui";

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const layers = () => document.querySelectorAll(".dialog-layer").length;
const button = (label: string) =>
  [...document.querySelectorAll<HTMLButtonElement>(".dialog-responses button")].find((b) => b.textContent === label)!;
const press = (key: string) =>
  document.body.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));

/** A promise and the functions that settle it. */
function pending<T = boolean | void>() {
  let resolve!: (value: T) => void;
  let reject!: (err: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

/** A dialog with a name field, a status line and an extra button, in busy mode `busy`. */
function open(onResponse: (id: string) => boolean | void | Promise<boolean | void>, busy: DialogBusy | null = {}) {
  const extra = document.createElement("div");
  extra.innerHTML = `<input id="name" /><select id="kind"><option>a</option></select>` +
    `<button id="guide">Open Guide</button><button id="more" disabled>More</button><div class="field-message"></div>`;
  const status = extra.querySelector<HTMLElement>(".field-message")!;
  const dialog = openDialog({
    heading: "Test",
    extra,
    responses: [{ id: "cancel", label: "Cancel" }, { id: "ok", label: "OK", style: "suggested" }],
    defaultResponse: "ok",
    busy: busy ? { status, ...busy } : undefined,
    onResponse,
  });
  const $ = <T extends HTMLElement>(sel: string) => extra.querySelector<T>(sel)!;
  return { dialog, status, name: $<HTMLInputElement>("#name"), kind: $<HTMLSelectElement>("#kind"), guide: $<HTMLButtonElement>("#guide"), more: $<HTMLButtonElement>("#more") };
}

afterEach(() => {
  document.querySelectorAll(".dialog-layer").forEach((layer) => layer.remove());
  document.body.querySelector("#toast-overlay")?.remove();
});

test("without busy mode a pending response leaves the dialog as it was", async () => {
  const work = pending();
  const onResponse = vi.fn((id: string) => (id === "ok" ? work.promise : undefined));
  const { name } = open(onResponse, null);
  press("Enter");
  press("Enter");
  expect(onResponse).toHaveBeenCalledTimes(2);
  expect(button("Cancel").disabled).toBe(false);
  expect(name.disabled).toBe(false);
  work.resolve(false);
  await settle();
  expect(layers()).toBe(1);
});

test("a pending response is the one action in flight: the rest is disabled until it settles", async () => {
  const work = pending();
  const onResponse = vi.fn((id: string) => (id === "ok" ? work.promise : undefined));
  const { dialog, name, kind, guide, more } = open(onResponse);
  press("Enter");
  expect(dialog.busy).toBe(true);
  expect([button("OK").disabled, button("Cancel").disabled, name.disabled, kind.disabled, guide.disabled]).toEqual([true, true, true, true, true]);
  press("Enter");
  press("Escape");
  button("OK").click();
  expect(onResponse).toHaveBeenCalledTimes(1);
  work.resolve(false);
  await settle();
  expect(dialog.busy).toBe(false);
  expect(layers()).toBe(1);
  expect([button("OK").disabled, button("Cancel").disabled, name.disabled, kind.disabled, guide.disabled]).toEqual([false, false, false, false, false]);
  expect(more.disabled).toBe(true);
});

test("a response that settles without false closes the dialog", async () => {
  const work = pending();
  open(() => work.promise);
  press("Enter");
  expect(layers()).toBe(1);
  work.resolve(undefined);
  await settle();
  expect(layers()).toBe(0);
});

test("live responses and controls stay usable, and a live Cancel closes while the work runs", async () => {
  const work = pending();
  const onResponse = vi.fn((id: string) => (id === "ok" ? work.promise : undefined));
  const extra = document.createElement("div");
  extra.innerHTML = `<div class="links"><button>Open Guide</button></div><button id="change">Change</button>`;
  const links = extra.querySelector<HTMLElement>(".links")!;
  const dialog = openDialog({
    heading: "Live",
    extra,
    responses: [{ id: "cancel", label: "Cancel" }, { id: "ok", label: "OK" }],
    defaultResponse: "ok",
    busy: { liveWhileBusy: ["cancel", links] },
    onResponse,
  });
  press("Enter");
  expect(button("OK").disabled).toBe(true);
  expect(button("Cancel").disabled).toBe(false);
  expect(links.querySelector("button")!.disabled).toBe(false);
  expect(extra.querySelector<HTMLButtonElement>("#change")!.disabled).toBe(true);
  press("Escape");
  await settle();
  expect(onResponse).toHaveBeenLastCalledWith("cancel");
  expect(layers()).toBe(0);
  expect(dialog.busy).toBe(true);
  work.resolve(false);
  await settle();
  expect(dialog.busy).toBe(false);
});

test("a live response that returns a promise while busy is awaited, not made a second action", async () => {
  const work = pending();
  const cancel = pending<void>();
  const dialog = openDialog({
    heading: "Live promise",
    responses: [{ id: "cancel", label: "Cancel" }, { id: "ok", label: "OK" }],
    busy: { liveWhileBusy: ["cancel"] },
    onResponse: (id) => (id === "ok" ? work.promise : cancel.promise),
  });
  button("OK").click();
  button("Cancel").click();
  expect(layers()).toBe(1);
  cancel.resolve();
  await settle();
  expect(layers()).toBe(0);
  expect(dialog.busy).toBe(true);
  work.resolve(false);
  await settle();
  expect(dialog.busy).toBe(false);
});

test("the wanted state of each response is kept while busy and applied afterwards", async () => {
  const work = pending();
  const { dialog } = open((id) => (id === "ok" ? work.promise : undefined));
  dialog.setResponseEnabled("cancel", false);
  expect(button("Cancel").disabled).toBe(true);
  dialog.setResponseEnabled("cancel", true);
  press("Enter");
  dialog.setResponseEnabled("ok", false);
  dialog.setResponseEnabled("cancel", true);
  expect(button("Cancel").disabled).toBe(true);
  work.resolve(false);
  await settle();
  expect(button("OK").disabled).toBe(true);
  expect(button("Cancel").disabled).toBe(false);
  // A disabled default response can't start anything.
  press("Enter");
  expect(dialog.busy).toBe(false);
});

test("a rejection is shown in the status line, cleaned, and the dialog stays open", async () => {
  const { dialog, status, name } = open(async (id) => {
    if (id !== "ok") return;
    dialog.setStatus("Working…");
    throw "Failed to write: Permission denied (os error 13)";
  });
  press("Enter");
  expect(status.textContent).toBe("Working…");
  expect(status.classList.contains("error")).toBe(false);
  await settle();
  expect(layers()).toBe(1);
  expect(status.textContent).toBe("Permission denied");
  expect(status.classList.contains("error")).toBe(true);
  expect(name.disabled).toBe(false);
  dialog.setStatus("Again…");
  expect(status.classList.contains("error")).toBe(false);
});

test("errorText words a rejection, and without a status line it is a toast", async () => {
  const { status } = open(async () => { throw new Error("No space left"); }, { errorText: (err) => `Oops: ${(err as Error).message}` });
  press("Enter");
  await settle();
  expect(status.textContent).toBe("Oops: No space left");
  document.querySelector(".dialog-layer")!.remove();

  const overlay = document.createElement("div");
  overlay.id = "toast-overlay";
  document.body.appendChild(overlay);
  openDialog({ heading: "No status", responses: [{ id: "ok", label: "OK" }], defaultResponse: "ok", busy: {}, onResponse: async () => { throw "Failed: Disk full"; } });
  press("Enter");
  await settle();
  expect(overlay.textContent).toContain("Disk full");
  expect(layers()).toBe(1);
});

test("run() is the action in flight for work started outside onResponse", async () => {
  const { dialog, status, name } = open(() => false);
  const work = pending<string>();
  const first = dialog.run(() => work.promise, "Connecting…");
  expect(dialog.busy).toBe(true);
  expect(status.textContent).toBe("Connecting…");
  expect(name.disabled).toBe(true);
  const second = vi.fn(async () => "again");
  expect(await dialog.run(second, "Again…")).toBeUndefined();
  expect(second).not.toHaveBeenCalled();
  expect(status.textContent).toBe("Connecting…");
  press("Enter");
  work.resolve("connected");
  expect(await first).toBe("connected");
  expect(dialog.busy).toBe(false);
  expect(name.disabled).toBe(false);
  expect(layers()).toBe(1);

  expect(await dialog.run(async () => { throw "Failed: Connection refused (os error 111)"; })).toBeUndefined();
  expect(status.textContent).toBe("Connection refused");
  expect(status.textContent).toBe(cleanError("Failed: Connection refused (os error 111)"));
});

test("a control that dropped focus when it was disabled gets it back", async () => {
  const work = pending();
  const { name } = open(() => work.promise);
  name.focus();
  press("Enter");
  // A browser drops focus to the page when a focused control is disabled;
  // jsdom doesn't, so drop it the way removing the focused element does.
  const outside = document.createElement("button");
  document.body.appendChild(outside);
  outside.focus();
  outside.remove();
  expect(document.activeElement).toBe(document.body);
  work.resolve(false);
  await settle();
  expect(document.activeElement).toBe(name);
});

test("focus that moved on to a prompt the work opened stays there", async () => {
  const { dialog, name } = open(() => false);
  name.focus();
  let prompt!: HTMLInputElement;
  await dialog.run(async () => {
    const extra = document.createElement("div");
    extra.innerHTML = `<input id="prompt" />`;
    openDialog({ heading: "Prompt", extra, responses: [{ id: "cancel", label: "Cancel" }] });
    prompt = extra.querySelector<HTMLInputElement>("#prompt")!;
  });
  expect(document.activeElement).toBe(prompt);
});

test("work started from a nested prompt gives focus back to the field that had it before the prompt", async () => {
  const { dialog, name } = open(() => false);
  name.focus();
  openDialog({
    heading: "Prompt",
    responses: [{ id: "cancel", label: "Cancel" }, { id: "go", label: "Go" }],
    onResponse: (id) => {
      if (id === "go") void dialog.run(async () => { await settle(); throw "Failed: Connection refused"; });
    },
  });
  expect(name.contains(document.activeElement)).toBe(false);
  // The prompt closes onto the field, which the work has just disabled.
  button("Go").click();
  expect(name.disabled).toBe(true);
  await settle();
  await settle();
  expect(name.disabled).toBe(false);
  expect(document.activeElement).toBe(name);
});

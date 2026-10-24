// Boot the real app (src/main.ts) in jsdom against dev/mock-tauri, and drive
// it with events. Each test file gets a fresh jsdom and module graph, so boot
// once per file; tests in a file share the app and the fake filesystem.
import { readFileSync } from "node:fs";
import { join } from "node:path";

// Vitest runs from the repo root.
const html = readFileSync(join(process.cwd(), "index.html"), "utf8");
const body = html.slice(html.indexOf("<body>") + "<body>".length, html.indexOf("</body>"));

/** Wait for pending backend calls and renders to finish. */
export async function settle(rounds = 6): Promise<void> {
  for (let i = 0; i < rounds; i++) await new Promise((r) => setTimeout(r, 0));
}

export async function boot(): Promise<void> {
  document.body.innerHTML = body.replace(/<script[\s\S]*?<\/script>/g, "");
  await import("../src/main.ts");
  window.dispatchEvent(new Event("DOMContentLoaded"));
  await settle(20);
}

export const $ = <T extends HTMLElement = HTMLElement>(sel: string) => document.querySelector<T>(sel)!;
export const $$ = (sel: string) => [...document.querySelectorAll<HTMLElement>(sel)];

const nameOf = (el: HTMLElement) => el.querySelector(".name")?.textContent ?? "";
const rendered = () =>
  $$("#view .item[data-index]").sort((a, b) => Number(a.dataset.index) - Number(b.dataset.index));

/** Names of the items currently rendered in the view, in display order. */
export function names(): string[] {
  return rendered().map(nameOf);
}

export function item(name: string): HTMLElement {
  const found = rendered().find((el) => nameOf(el) === name);
  if (!found) throw new Error(`no item “${name}” in ${JSON.stringify(names())}`);
  return found;
}

export function selectedNames(): string[] {
  return rendered().filter((el) => el.classList.contains("selected")).map(nameOf);
}

export function cursorName(): string | undefined {
  const el = rendered().find((el) => el.classList.contains("cursor"));
  return el ? nameOf(el) : undefined;
}

export function prefs(): Record<string, unknown> {
  return JSON.parse(localStorage.getItem("prefs") ?? "{}");
}

/** Visible dialog heading, if a dialog is open. */
export function dialogHeading(): string | undefined {
  return document.querySelector(".dialog-layer .dialog-heading")?.textContent ?? undefined;
}

export async function closeOverlays(): Promise<void> {
  for (let i = 0; i < 3 && (document.querySelector(".dialog-layer, .popover-layer")); i++) {
    await key("Escape", { target: document.querySelector(".dialog-layer, .popover-layer") ?? document.body });
  }
}

const isMac = true;
/** Press a key on `target` (default: the view). `mod` is ⌘ on the Mac code path. */
export async function key(k: string, opts: { mod?: boolean; shift?: boolean; alt?: boolean; ctrl?: boolean; target?: EventTarget } = {}): Promise<KeyboardEvent> {
  const event = new KeyboardEvent("keydown", {
    key: k,
    bubbles: true,
    cancelable: true,
    metaKey: !!opts.mod && isMac,
    ctrlKey: !!opts.ctrl || (!!opts.mod && !isMac),
    shiftKey: !!opts.shift,
    altKey: !!opts.alt,
  });
  (opts.target ?? $("#view")).dispatchEvent(event);
  await settle();
  return event;
}

export async function mouse(target: Element, type: string, init: MouseEventInit = {}): Promise<void> {
  target.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, ...init }));
  await settle();
}

export async function click(target: Element, init: MouseEventInit = {}): Promise<void> {
  await mouse(target, "mousedown", init);
  await mouse(target, "mouseup", init);
  await mouse(target, "click", init);
}

/** Open a folder by double-clicking its item. */
export async function open(name: string): Promise<void> {
  await mouse(item(name), "dblclick");
  await settle(10);
}

export function menuLabels(): string[] {
  return $$(".popover .menu-item, .popover button").map((el) => el.textContent?.trim() ?? "").filter(Boolean);
}

export async function chooseMenu(label: string): Promise<void> {
  const button = $$(".popover .menu-item, .popover button").find((el) => el.textContent?.trim().startsWith(label));
  if (!button) throw new Error(`no menu item “${label}” in ${JSON.stringify(menuLabels())}`);
  await click(button);
  await settle(10);
}

export function title(): string {
  return document.title;
}

/** Click the dialog button labelled `label`. */
export async function respond(label: string): Promise<void> {
  const button = $$(".dialog-layer .dialog-responses button").find((b) => b.textContent?.trim() === label);
  if (!button) throw new Error(`no dialog button “${label}”`);
  await click(button);
  await settle(15);
}

/** Text of the toasts on screen. */
export function toasts(): string[] {
  return $$(".toast").map((t) => t.textContent ?? "");
}

/** Type into an input and fire `input`, like a user would. */
export async function type(input: HTMLInputElement, value: string): Promise<void> {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await settle();
}

/** Go to `path` through the path entry (⌘L, type, Enter). */
export async function goTo(path: string): Promise<void> {
  await key("l", { mod: true });
  await type($<HTMLInputElement>("#path-entry"), path);
  await key("Enter", { target: $("#path-entry") });
  await settle(15);
}

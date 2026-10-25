// Long operations (copies, moves, trash batches, Empty Trash, remote opens)
// and the header bar's operations button that shows them, like GNOME Files'
// (spec docs/superpowers/specs/2026-10-25-operation-progress-design.md).
//
// The model (titles, detail text, fractions) is pure so it can be tested on
// its own; the widget below renders it into the button and its popover.

import { formatSize } from "./format";
import { sym } from "./icons";
import { openPopover, type PopoverHandle } from "./ui";

/** The error a cancelled command ends with; a cancel, not a failure. */
export const CANCELLED = "Cancelled.";

export type OpKind = "copy" | "move" | "trash" | "delete" | "empty" | "open";
export type OpState = "waiting" | "preparing" | "working" | "cancelling";

/** What the backend streams for one command (mirrors Rust's ProgressUpdate). */
export interface ProgressUpdate {
  bytes: number;
  items: number;
  total: number | null;
  phase: "preparing" | "working";
  done: boolean;
}

export interface Operation {
  id: string;
  title: string;
  state: OpState;
  /** Measured bytes for the whole operation; null when unknown. */
  totalBytes: number | null;
  totalItems: number;
  /** Bytes and items of the items already finished. */
  doneBytes: number;
  doneItems: number;
  /** The running item's latest update. */
  current: ProgressUpdate | null;
  startedAt: number;
  /** Smoothed speed in bytes per millisecond, once there are two samples. */
  speed: number | null;
  sample?: { at: number; bytes: number };
  cancel(): void;
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

const VERBS: Record<Exclude<OpKind, "empty">, string> = {
  copy: "Copying",
  move: "Moving",
  trash: "Moving",
  delete: "Deleting",
  open: "Opening",
};

/** "Copying “a.txt”", "Copying 3 files to “Documents”", "Emptying the Trash", … */
export function opTitle(kind: OpKind, names: string[], destName?: string): string {
  if (kind === "empty") return "Emptying the Trash";
  const what = names.length === 1 ? `“${names[0]}”` : `${names.length} files`;
  const title = `${VERBS[kind]} ${what}`;
  if (kind === "trash") return `${title} to the Trash`;
  if (names.length > 1 && destName && (kind === "copy" || kind === "move")) return `${title} to “${destName}”`;
  return title;
}

function bytesDone(op: Operation): number {
  return op.doneBytes + (op.current?.bytes ?? 0);
}

function itemsDone(op: Operation): number {
  return op.doneItems + (op.current?.items ?? 0);
}

/** 0–1: bytes when there is a byte total, else items. */
export function opFraction(op: Operation): number {
  if (op.totalBytes) return Math.min(1, bytesDone(op) / op.totalBytes);
  if (op.totalItems) return Math.min(1, itemsDone(op) / op.totalItems);
  return 0;
}

/** "59 seconds left", "1 minute left", "2 hours left". */
function timeLeft(ms: number): string {
  const seconds = Math.max(1, Math.round(ms / 1000));
  if (seconds < 60) return seconds === 1 ? "1 second left" : `${seconds} seconds left`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return minutes === 1 ? "1 minute left" : `${minutes} minutes left`;
  const hours = Math.round(minutes / 60);
  return hours === 1 ? "1 hour left" : `${hours} hours left`;
}

/** Time left is only guessed after this much work. */
const SETTLE_MS = 2000;

/** The popover row's second line. */
export function opDetail(op: Operation, now: number): string {
  if (op.state === "waiting") return "Waiting…";
  if (op.state === "preparing") return "Preparing…";
  if (op.state === "cancelling") return "Cancelling…";
  if (!op.totalBytes) return `${itemsDone(op)} of ${op.totalItems} files`;
  const done = bytesDone(op);
  const sizes = `${formatSize(done)} of ${formatSize(op.totalBytes)}`;
  const elapsed = now - op.startedAt;
  const speed = op.speed ?? (elapsed > 0 ? done / elapsed : 0);
  if (elapsed < SETTLE_MS || speed <= 0) return sizes;
  return `${sizes} — ${timeLeft((op.totalBytes - done) / speed)}`;
}

/** Fold a backend update into `op`, smoothing the speed. */
export function updateOperation(op: Operation, update: ProgressUpdate, now = Date.now()): void {
  op.current = update;
  if (op.state === "preparing") op.state = "working";
  const done = bytesDone(op);
  if (op.sample && now > op.sample.at) {
    const instant = Math.max(0, done - op.sample.bytes) / (now - op.sample.at);
    op.speed = op.speed === null ? instant : 0.7 * op.speed + 0.3 * instant;
  }
  op.sample = { at: now, bytes: done };
  render();
}

/** The running item finished: keep its bytes and items, or count one item. */
export function finishItem(op: Operation): void {
  if (op.current) {
    op.doneBytes += op.current.bytes;
    op.doneItems += Math.max(1, op.current.items);
  } else {
    op.doneItems += 1;
  }
  op.current = null;
  if (op.state === "preparing") op.state = "working";
  render();
}

export function setOperationState(op: Operation, state: OpState): void {
  op.state = state;
  if (state === "working" && op.startedAt === 0) op.startedAt = Date.now();
  render();
}

// ---------------------------------------------------------------------------
// Widget: the header bar button and its popover
// ---------------------------------------------------------------------------

let ops: Operation[] = [];
let opSeq = 0;
let button: HTMLElement | null = null;
let popover: PopoverHandle | null = null;
let autoOpened = false;
let hideTimer: ReturnType<typeof setTimeout> | undefined;
/** The button shows a checkmark until then, after the last operation ended. */
let lingerUntil = 0;
/** The button stays this long after the last operation ends. */
const LINGER_MS = 3000;
/** An operation still running after this long opens the popover, once. */
const AUTO_OPEN_MS = 1000;

export function startOperation(
  title: string,
  opts: { totalBytes?: number | null; totalItems: number; onCancel: () => void; waiting?: boolean }
): Operation {
  const op: Operation = {
    id: `op-${Date.now().toString(36)}-${++opSeq}`,
    title,
    state: opts.waiting ? "waiting" : "preparing",
    totalBytes: opts.totalBytes ?? null,
    totalItems: opts.totalItems,
    doneBytes: 0,
    doneItems: 0,
    current: null,
    startedAt: opts.waiting ? 0 : Date.now(),
    speed: null,
    cancel: () => {
      if (op.state === "cancelling") return;
      op.state = "cancelling";
      render();
      opts.onCancel();
    },
  };
  ops.push(op);
  clearTimeout(hideTimer);
  lingerUntil = 0;
  setTimeout(() => {
    if (!autoOpened && ops.includes(op) && button) {
      autoOpened = true;
      openOperationsPopover();
    }
  }, AUTO_OPEN_MS);
  render();
  return op;
}

/** Remove a finished, failed or cancelled operation. */
export function endOperation(op: Operation): void {
  ops = ops.filter((o) => o !== op);
  if (ops.length === 0) {
    // Linger with a checkmark, then hide (unless the popover is open).
    lingerUntil = Date.now() + LINGER_MS;
    clearTimeout(hideTimer);
    hideTimer = setTimeout(render, LINGER_MS);
  }
  render();
}

export function initOperationsButton(el: HTMLElement): void {
  button = el;
  button.addEventListener("click", () => (popover ? popover.close() : openOperationsPopover()));
  render();
}

function openOperationsPopover(): void {
  if (!button || popover) return;
  const content = document.createElement("div");
  content.className = "operations-popover";
  popover = openPopover(button, content, {
    onClose: () => {
      popover = null;
      render();
    },
  });
  render();
}

/** Overall fraction across every operation, by item weight. */
function overall(): number {
  if (ops.length === 0) return 1;
  return ops.reduce((sum, op) => sum + opFraction(op), 0) / ops.length;
}

/** A symbolic pie filled to `fraction` (drawn here; it changes every update). */
function pie(fraction: number): string {
  const r = 5.5;
  const c = 2 * Math.PI * r;
  return `<svg class="sym op-pie" viewBox="0 0 16 16" aria-hidden="true">` +
    `<circle class="op-pie-track" cx="8" cy="8" r="${r}"/>` +
    `<circle class="op-pie-fill" cx="8" cy="8" r="${r}" stroke-dasharray="${(fraction * c).toFixed(2)} ${c.toFixed(2)}" transform="rotate(-90 8 8)"/>` +
    `</svg>`;
}

function rowFor(op: Operation, now: number): HTMLElement {
  const row = document.createElement("div");
  row.className = "operation-row";
  const text = document.createElement("div");
  text.className = "operation-text";
  const title = document.createElement("div");
  title.className = "operation-title";
  title.textContent = op.title;
  const bar = document.createElement("div");
  bar.className = "operation-bar";
  const fill = document.createElement("div");
  fill.className = "operation-bar-fill";
  fill.style.width = `${(opFraction(op) * 100).toFixed(1)}%`;
  bar.appendChild(fill);
  const detail = document.createElement("div");
  detail.className = "operation-detail";
  detail.textContent = opDetail(op, now);
  text.append(title, bar, detail);
  const cancel = document.createElement("button");
  cancel.className = "flat-btn operation-cancel";
  cancel.title = "Cancel";
  cancel.setAttribute("aria-label", `Cancel ${op.title}`);
  cancel.innerHTML = sym("window-close");
  cancel.disabled = op.state === "cancelling";
  cancel.addEventListener("click", () => op.cancel());
  row.append(text, cancel);
  return row;
}

function render(): void {
  if (!button) return;
  button.hidden = ops.length === 0 && !popover && Date.now() >= lingerUntil;
  button.innerHTML = ops.length ? pie(overall()) : sym("object-select");
  button.title = ops.length ? "Show Operations" : "Operations Finished";
  if (popover) {
    const now = Date.now();
    const list = popover.element.querySelector(".operations-popover") ?? popover.element;
    list.replaceChildren(...ops.map((op) => rowFor(op, now)));
    if (ops.length === 0) {
      const empty = document.createElement("div");
      empty.className = "operation-empty";
      empty.textContent = "All operations finished";
      list.appendChild(empty);
    }
  }
}

/** For tests: forget every operation and hide the button. */
export function resetOperations(): void {
  ops = [];
  autoOpened = false;
  lingerUntil = 0;
  clearTimeout(hideTimer);
  hideTimer = undefined;
  popover?.close();
  popover = null;
  render();
}

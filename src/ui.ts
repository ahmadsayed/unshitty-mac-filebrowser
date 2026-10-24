// Small libadwaita-style widgets: popovers, popover menus, dialogs, toasts.
import { sym } from "./icons";

// ---------------------------------------------------------------------------
// Popovers
// ---------------------------------------------------------------------------

export type Anchor = HTMLElement | { x: number; y: number };

export interface PopoverHandle {
  close(): void;
  element: HTMLElement;
}

let activePopover: PopoverHandle | null = null;

export function closePopover(): void {
  activePopover?.close();
}

export function hasOpenOverlay(): boolean {
  return activePopover !== null || document.querySelector(".dialog-layer") !== null;
}

/**
 * Show `content` in a popover. An element anchor gets an arrow and is
 * centred under the element (GTK menu-button popovers); a point anchor opens
 * at the pointer without an arrow (context menus).
 */
export function openPopover(
  anchor: Anchor,
  content: HTMLElement,
  opts: { onClose?: () => void; focus?: HTMLElement } = {}
): PopoverHandle {
  closePopover();

  const layer = document.createElement("div");
  layer.className = "popover-layer";
  const pop = document.createElement("div");
  pop.className = "popover";
  pop.appendChild(content);
  layer.appendChild(pop);
  document.body.appendChild(layer);

  const isElement = anchor instanceof HTMLElement;
  if (!isElement) {
    // Point anchors are context menus. They often replace a menu that has
    // just been dismissed, so replaying the fade makes the menu blink at
    // (nearly) the same spot; open them instantly like a native menu.
    pop.classList.add("no-animation");
  }
  if (isElement) anchor.classList.add("open");

  // Position: below the anchor when it fits, otherwise above; clamp to window.
  const margin = 6;
  const pw = pop.offsetWidth;
  const ph = pop.offsetHeight;
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  let left: number;
  let top: number;
  if (isElement) {
    const r = anchor.getBoundingClientRect();
    const cx = r.left + r.width / 2;
    left = Math.min(Math.max(cx - pw / 2, margin), vw - pw - margin);
    const below = r.bottom + 10 + ph <= vh - margin;
    top = below ? r.bottom + 10 : r.top - 10 - ph;
    pop.classList.add(below ? "below" : "above");
    const arrow = document.createElement("div");
    arrow.className = "popover-arrow";
    arrow.style.left = `${Math.min(Math.max(cx - left - 7, 14), pw - 28)}px`;
    pop.prepend(arrow);
  } else {
    left = Math.min(anchor.x, vw - pw - margin);
    top = anchor.y + ph <= vh - margin ? anchor.y : Math.max(anchor.y - ph, margin);
    pop.style.transformOrigin = "top left";
  }
  pop.style.left = `${Math.max(left, margin)}px`;
  pop.style.top = `${Math.max(top, margin)}px`;

  const previousFocus = document.activeElement as HTMLElement | null;

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      handle.close();
    }
  };

  const handle: PopoverHandle = {
    element: pop,
    close() {
      if (activePopover !== handle) return;
      activePopover = null;
      document.removeEventListener("keydown", onKey, true);
      layer.remove();
      if (isElement) anchor.classList.remove("open");
      opts.onClose?.();
      previousFocus?.focus?.({ preventScroll: true });
    },
  };

  // A press outside the popover closes it (and is swallowed, like GTK).
  layer.addEventListener("mousedown", (e) => {
    if (!pop.contains(e.target as Node)) {
      e.preventDefault();
      handle.close();
    }
  });
  layer.addEventListener("contextmenu", (e) => {
    if (!pop.contains(e.target as Node)) {
      e.preventDefault();
      handle.close();
    }
  });
  document.addEventListener("keydown", onKey, true);

  activePopover = handle;
  (opts.focus ?? pop.querySelector<HTMLElement>("button:not(:disabled), input"))?.focus({ preventScroll: true });
  return handle;
}

// ---------------------------------------------------------------------------
// Popover menus
// ---------------------------------------------------------------------------

export type MenuItem =
  | { separator: true }
  | { heading: string }
  | {
      label: string;
      action?: () => void;
      disabled?: boolean;
      /** Render as a check item with this state. */
      check?: boolean;
      /** Render as a radio item with this state. */
      radio?: boolean;
      accel?: string;
    };

export function openMenu(anchor: Anchor, items: MenuItem[]): PopoverHandle {
  const menu = document.createElement("div");
  menu.className = "menu";
  menu.setAttribute("role", "menu");

  const hasIndicators = items.some((i) => "label" in i && (i.check !== undefined || i.radio !== undefined));
  let handle: PopoverHandle | null = null;

  for (const item of items) {
    if ("separator" in item) {
      const sep = document.createElement("div");
      sep.className = "menu-separator";
      menu.appendChild(sep);
      continue;
    }
    if ("heading" in item) {
      const h = document.createElement("div");
      h.className = "menu-heading";
      h.textContent = item.heading;
      menu.appendChild(h);
      continue;
    }
    const btn = document.createElement("button");
    btn.className = "menu-item";
    btn.setAttribute("role", item.radio !== undefined ? "menuitemradio" : item.check !== undefined ? "menuitemcheckbox" : "menuitem");
    btn.disabled = !!item.disabled;
    let indicator = "";
    if (hasIndicators) {
      const mark = item.check ? sym("object-select") : item.radio ? `<span class="radio-dot"></span>` : "";
      indicator = `<span class="indicator">${mark}</span>`;
    }
    btn.innerHTML = `${indicator}<span class="label"></span>${item.accel ? `<span class="accel"></span>` : ""}`;
    btn.querySelector(".label")!.textContent = item.label;
    if (item.accel) btn.querySelector(".accel")!.textContent = item.accel;
    btn.addEventListener("click", () => {
      handle?.close();
      item.action?.();
    });
    menu.appendChild(btn);
  }

  // Arrow-key navigation between enabled items.
  menu.addEventListener("keydown", (e) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const buttons = [...menu.querySelectorAll<HTMLButtonElement>(".menu-item:not(:disabled)")];
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = e.key === "ArrowDown" ? (i + 1) % buttons.length : (i - 1 + buttons.length) % buttons.length;
    buttons[next]?.focus();
  });

  handle = openPopover(anchor, menu, { focus: menu });
  return handle;
}

// ---------------------------------------------------------------------------
// Dialogs (AdwAlertDialog)
// ---------------------------------------------------------------------------

export interface DialogResponse {
  id: string;
  label: string;
  style?: "suggested" | "destructive";
}

export interface DialogHandle {
  close(): void;
  setResponseEnabled(id: string, enabled: boolean): void;
}

export function openDialog(opts: {
  heading: string;
  body?: string;
  extra?: HTMLElement;
  /** Extra class on `.dialog`, for wider or differently laid-out dialogs. */
  className?: string;
  responses: DialogResponse[];
  defaultResponse?: string;
  closeResponse?: string;
  /** Return false to keep the dialog open. */
  onResponse?: (id: string) => boolean | void | Promise<boolean | void>;
}): DialogHandle {
  closePopover();
  const layer = document.createElement("div");
  layer.className = "dialog-layer";
  const dialog = document.createElement("div");
  dialog.className = "dialog";
  if (opts.className) dialog.classList.add(opts.className);
  dialog.setAttribute("role", "dialog");
  dialog.setAttribute("aria-modal", "true");

  const body = document.createElement("div");
  body.className = "dialog-body";
  const heading = document.createElement("h2");
  heading.className = "dialog-heading";
  heading.textContent = opts.heading;
  body.appendChild(heading);
  if (opts.body) {
    const text = document.createElement("p");
    text.className = "dialog-text";
    text.textContent = opts.body;
    body.appendChild(text);
  }
  if (opts.extra) {
    opts.extra.classList.add("dialog-extra");
    body.appendChild(opts.extra);
  }
  dialog.appendChild(body);

  const responses = document.createElement("div");
  responses.className = "dialog-responses";
  const buttons = new Map<string, HTMLButtonElement>();
  for (const r of opts.responses) {
    const b = document.createElement("button");
    b.className = `btn pill ${r.style ?? ""}`;
    b.textContent = r.label;
    b.addEventListener("click", () => respond(r.id));
    responses.appendChild(b);
    buttons.set(r.id, b);
  }
  dialog.appendChild(responses);
  layer.appendChild(dialog);
  document.body.appendChild(layer);

  const previousFocus = document.activeElement as HTMLElement | null;
  const closeId = opts.closeResponse ?? "cancel";

  async function respond(id: string) {
    const btn = buttons.get(id);
    if (btn?.disabled) return;
    const keep = (await opts.onResponse?.(id)) === false;
    if (!keep) handle.close();
  }

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      respond(closeId);
    } else if (e.key === "Enter" && opts.defaultResponse) {
      e.preventDefault();
      e.stopPropagation();
      respond(opts.defaultResponse);
    }
  };
  document.addEventListener("keydown", onKey, true);

  const handle: DialogHandle = {
    close() {
      document.removeEventListener("keydown", onKey, true);
      layer.remove();
      previousFocus?.focus?.({ preventScroll: true });
    },
    setResponseEnabled(id, enabled) {
      const b = buttons.get(id);
      if (b) b.disabled = !enabled;
    },
  };

  (dialog.querySelector<HTMLElement>("input") ?? (opts.defaultResponse ? buttons.get(opts.defaultResponse) : null) ?? dialog.querySelector("button"))?.focus();
  return handle;
}

// ---------------------------------------------------------------------------
// Toasts (AdwToast)
// ---------------------------------------------------------------------------

export function showToast(
  title: string,
  opts: { button?: { label: string; onClick: () => void }; timeout?: number } = {}
): void {
  const overlay = document.querySelector<HTMLElement>("#toast-overlay");
  if (!overlay) return;
  // libadwaita shows one toast at a time; a new one replaces the current.
  overlay.querySelectorAll(".toast").forEach((t) => t.remove());

  const toast = document.createElement("div");
  toast.className = "toast";
  toast.setAttribute("role", "status");
  const label = document.createElement("span");
  label.className = "toast-title";
  label.textContent = title;
  toast.appendChild(label);

  const dismiss = () => {
    toast.classList.add("leaving");
    setTimeout(() => toast.remove(), 150);
  };

  if (opts.button) {
    const b = document.createElement("button");
    b.className = "btn";
    b.textContent = opts.button.label;
    b.addEventListener("click", () => {
      dismiss();
      opts.button!.onClick();
    });
    toast.appendChild(b);
  }
  const close = document.createElement("button");
  close.className = "flat-btn";
  close.setAttribute("aria-label", "Dismiss");
  close.innerHTML = sym("window-close");
  close.addEventListener("click", dismiss);
  toast.appendChild(close);

  overlay.appendChild(toast);
  setTimeout(dismiss, opts.timeout ?? 5000);
}

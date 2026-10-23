import { homeDir, documentDir, downloadDir, audioDir, pictureDir, videoDir } from "@tauri-apps/api/path";
import { listDir, makeDir, renameEntry, type Entry } from "./api";
import { folderIcon, fileIcon } from "./icons";

// ---------------------------------------------------------------------------
// Module-level state
// ---------------------------------------------------------------------------
export let currentPath = "";
export type ViewMode = "list" | "icon";
export let viewMode: ViewMode = "icon";
let showHidden = false;

// History stacks: back contains paths we navigated away from; forward contains
// paths we went back from.
let backStack: string[] = [];
let forwardStack: string[] = [];

export function setViewMode(mode: ViewMode): void {
  viewMode = mode;
}

// ---------------------------------------------------------------------------
// DOM refs (set in init)
// ---------------------------------------------------------------------------
let listingEl: HTMLElement | null = null;
let crumbSegmentsEl: HTMLElement | null = null;
let crumbbarEl: HTMLElement | null = null;
let btnBack: HTMLButtonElement | null = null;
let btnForward: HTMLButtonElement | null = null;
let errorBannerEl: HTMLElement | null = null;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function formatDate(unixSeconds: number): string {
  if (unixSeconds === 0) return "—";
  return new Date(unixSeconds * 1000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

// SECURITY: escape HTML entities in user-derived strings before innerHTML insertion
export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function sortEntries(entries: Entry[]): Entry[] {
  return [...entries].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

// ---------------------------------------------------------------------------
// Error banner
// ---------------------------------------------------------------------------

function showErrorBanner(message: string): void {
  if (!errorBannerEl) return;
  errorBannerEl.textContent = message;
  errorBannerEl.classList.add("visible");
}

function hideErrorBanner(): void {
  if (!errorBannerEl) return;
  errorBannerEl.classList.remove("visible");
}

// ---------------------------------------------------------------------------
// History stack management
// ---------------------------------------------------------------------------

function updateNavButtons(): void {
  if (btnBack) btnBack.disabled = backStack.length === 0;
  if (btnForward) btnForward.disabled = forwardStack.length === 0;
}

// ---------------------------------------------------------------------------
// Breadcrumb (normal mode)
// ---------------------------------------------------------------------------
function buildBreadcrumb(path: string): void {
  if (!crumbbarEl || !crumbSegmentsEl) return;

  // Ensure we're showing the crumb bar (not the path input)
  const existingInput = crumbbarEl.querySelector<HTMLInputElement>(".path-input");
  if (existingInput) {
    existingInput.remove();
  }
  crumbSegmentsEl.style.display = "";

  const crumbHome = crumbbarEl.querySelector<HTMLElement>("#crumb-home");
  if (crumbHome) crumbHome.style.display = "";

  const parts = path.replace(/^\//, "").split("/").filter(Boolean);

  if (parts.length === 0) {
    crumbSegmentsEl.innerHTML = "";
    return;
  }

  const frags: string[] = [];
  for (let i = 0; i < parts.length; i++) {
    if (i > 0) {
      frags.push(`<span class="crumb-sep" aria-hidden="true">›</span>`);
    }
    // Build the path up to this segment
    const segPath = "/" + parts.slice(0, i + 1).join("/");
    // SECURITY: escape each path segment before insertion
    frags.push(
      `<span class="crumb-seg" data-path="${escapeHtml(segPath)}" role="button" tabindex="0">${escapeHtml(parts[i])}</span>`
    );
  }
  crumbSegmentsEl.innerHTML = frags.join("");

  // Wire up click handlers on each crumb segment
  crumbSegmentsEl.querySelectorAll<HTMLElement>(".crumb-seg").forEach((seg) => {
    const segPath = seg.dataset.path;
    if (!segPath) return;
    seg.addEventListener("click", () => {
      navigate(segPath).catch(console.error);
    });
    seg.addEventListener("keydown", (e: KeyboardEvent) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        navigate(segPath).catch(console.error);
      }
    });
  });
}

// ---------------------------------------------------------------------------
// Editable path bar (Cmd+L / Ctrl+L)
// ---------------------------------------------------------------------------

function openPathInput(): void {
  if (!crumbbarEl || !crumbSegmentsEl) return;

  // Don't open if already open
  if (crumbbarEl.querySelector(".path-input")) return;

  // Hide breadcrumb content
  crumbSegmentsEl.style.display = "none";
  const crumbHome = crumbbarEl.querySelector<HTMLElement>("#crumb-home");
  if (crumbHome) crumbHome.style.display = "none";

  const input = document.createElement("input");
  input.type = "text";
  input.className = "path-input";
  input.value = currentPath;
  input.setAttribute("aria-label", "Path");
  input.setAttribute("spellcheck", "false");
  input.setAttribute("autocomplete", "off");
  crumbbarEl.appendChild(input);

  // Focus and select all text
  input.focus();
  input.select();

  const closeInput = (shouldNavigate: boolean) => {
    const typed = input.value.trim();
    if (input.parentNode) {
      input.remove();
    }
    crumbSegmentsEl!.style.display = "";
    if (crumbHome) crumbHome.style.display = "";

    if (shouldNavigate && typed) {
      navigate(typed).catch(console.error);
    } else {
      // Just rebuild the crumb for current path (revert)
      buildBreadcrumb(currentPath);
    }
  };

  input.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      const typed = input.value.trim();
      // Try navigating; if it fails, keep the input open with error banner
      navigateWithValidation(typed, input).catch(console.error);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeInput(false);
    }
  });

  // Blur = revert (click away)
  input.addEventListener("blur", () => {
    // Small timeout to allow Enter keydown to fire first
    setTimeout(() => {
      if (input.parentNode) {
        closeInput(false);
      }
    }, 100);
  });
}

async function navigateWithValidation(path: string, input: HTMLInputElement): Promise<void> {
  try {
    // Validate by trying to list the dir
    await listDir(path, showHidden);
    // Success — close input and navigate
    input.remove();
    const crumbSegmentsElLocal = document.querySelector<HTMLElement>("#crumb-segments");
    if (crumbSegmentsElLocal) crumbSegmentsElLocal.style.display = "";
    const crumbHome = crumbbarEl?.querySelector<HTMLElement>("#crumb-home");
    if (crumbHome) crumbHome.style.display = "";
    await navigate(path);
  } catch (err) {
    showErrorBanner(`Cannot navigate to "${escapeHtml(path)}": ${String(err)}`);
    // Keep input open
    input.focus();
    input.select();
  }
}

// ---------------------------------------------------------------------------
// Core navigate function (updates history) vs render (does not push history)
// ---------------------------------------------------------------------------

/**
 * Navigate to a new path. Pushes currentPath onto backStack and clears forwardStack.
 */
export async function navigate(path: string): Promise<void> {
  if (path === currentPath) return;
  if (currentPath) {
    backStack.push(currentPath);
  }
  forwardStack = [];
  await renderInternal(path);
  updateNavButtons();
}

/**
 * Core render: update currentPath and re-render the listing.
 * Does NOT manipulate history — called by navigate, back, forward.
 */
async function renderInternal(path: string): Promise<void> {
  if (!listingEl) return;

  currentPath = path;
  hideErrorBanner();

  // Update breadcrumb
  buildBreadcrumb(path);

  // Mark the matching sidebar item active
  document.querySelectorAll<HTMLElement>(".sidebar-item").forEach((item) => {
    item.classList.toggle("active", item.dataset.path === path);
  });

  listingEl.innerHTML = "";

  let entries: Entry[];
  try {
    entries = await listDir(path, showHidden);
  } catch (err) {
    const errEl = document.createElement("div");
    errEl.className = "listing-error";
    errEl.textContent = `Could not read directory: ${String(err)}`;
    listingEl.appendChild(errEl);
    return;
  }

  const sorted = sortEntries(entries);

  if (sorted.length === 0) {
    const empty = document.createElement("div");
    empty.className = "listing-empty";
    empty.textContent = "No items";
    listingEl.appendChild(empty);
    return;
  }

  const view =
    viewMode === "icon" ? renderIconView(sorted) : renderListView(sorted);
  listingEl.appendChild(view);
}

/**
 * Public render: navigates to the path (pushes history).
 * Preserved as the exported API for backward compat.
 */
export async function render(path: string): Promise<void> {
  await navigate(path);
}

// ---------------------------------------------------------------------------
// Render: list view
// ---------------------------------------------------------------------------
function renderListView(entries: Entry[]): HTMLElement {
  const container = document.createElement("div");
  container.className = "listing-list";

  const header = document.createElement("div");
  header.className = "list-header";
  header.innerHTML = `
    <span></span>
    <span>Name</span>
    <span style="text-align:right;padding-right:16px">Size</span>
    <span>Modified</span>
  `;
  container.appendChild(header);

  for (const entry of entries) {
    const row = document.createElement("div");
    row.className = "list-row";
    if (entry.is_dir) row.dataset.isDir = "true";
    row.dataset.path = entry.path;

    const sizeText = entry.is_dir ? "—" : formatSize(entry.size);
    const iconSvg = entry.is_dir ? folderIcon(entry.name) : fileIcon(entry);
    const nameHtml = escapeHtml(entry.name);

    row.innerHTML = `
      <span class="list-cell-icon">${iconSvg}</span>
      <span class="list-cell-name">${nameHtml}</span>
      <span class="list-cell-size">${sizeText}</span>
      <span class="list-cell-modified">${formatDate(entry.modified)}</span>
    `;

    row.addEventListener("click", () => {
      selectEntry(row);
    });

    row.addEventListener("dblclick", () => {
      if (entry.is_dir) {
        navigate(entry.path).catch(console.error);
      }
    });

    row.addEventListener("keydown", (e: KeyboardEvent) => {
      if (e.key === "F2" || (e.key === "Enter" && row.classList.contains("selected"))) {
        e.preventDefault();
        startRename(entry, row);
      }
    });

    container.appendChild(row);
  }

  return container;
}

// ---------------------------------------------------------------------------
// Render: grid / icon view
// ---------------------------------------------------------------------------
function renderIconView(entries: Entry[]): HTMLElement {
  const grid = document.createElement("div");
  grid.className = "icon-grid";

  for (const entry of entries) {
    const item = document.createElement("div");
    item.className = "icon-item";
    item.dataset.path = entry.path;
    if (entry.is_dir) item.dataset.isDir = "true";
    item.tabIndex = 0;

    const iconSvg = entry.is_dir ? folderIcon(entry.name) : fileIcon(entry);
    const nameHtml = escapeHtml(entry.name);

    item.innerHTML = `
      <span class="icon-item-icon">${iconSvg}</span>
      <span class="icon-item-name">${nameHtml}</span>
    `;

    item.addEventListener("click", () => {
      selectEntry(item);
    });

    item.addEventListener("dblclick", () => {
      if (entry.is_dir) {
        navigate(entry.path).catch(console.error);
      }
    });

    item.addEventListener("keydown", (e: KeyboardEvent) => {
      if (e.key === "F2" || (e.key === "Enter" && item.classList.contains("selected"))) {
        e.preventDefault();
        startRename(entry, item);
      }
    });

    grid.appendChild(item);
  }
  return grid;
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

let selectedEl: HTMLElement | null = null;

function selectEntry(el: HTMLElement): void {
  if (selectedEl) selectedEl.classList.remove("selected");
  selectedEl = el;
  el.classList.add("selected");
  el.focus();
}

// ---------------------------------------------------------------------------
// Rename (inline prompt)
// ---------------------------------------------------------------------------

function startRename(entry: Entry, _el: HTMLElement): void {
  // Show a simple prompt-style overlay inside the item
  const oldName = entry.name;
  const newName = window.prompt(`Rename "${oldName}" to:`, oldName);
  if (!newName || newName === oldName) return;

  const newPath = currentPath.replace(/\/$/, "") + "/" + newName;
  renameEntry(entry.path, newPath)
    .then(() => render(currentPath))
    .catch((err: unknown) => {
      showErrorBanner(`Rename failed: ${String(err)}`);
    });
}

// ---------------------------------------------------------------------------
// New folder dialog
// ---------------------------------------------------------------------------

function promptNewFolder(): void {
  const name = window.prompt("New folder name:");
  if (!name) return;

  const newPath = currentPath.replace(/\/$/, "") + "/" + name;
  makeDir(newPath)
    .then(() => render(currentPath))
    .catch((err: unknown) => {
      showErrorBanner(`Could not create folder: ${String(err)}`);
    });
}

// ---------------------------------------------------------------------------
// Sidebar helpers
// ---------------------------------------------------------------------------
function addSidebarItem(
  label: string,
  iconHtml: string,
  path: string,
  section: HTMLElement,
  isEmoji = false
): void {
  const item = document.createElement("div");
  item.className = "sidebar-item";
  item.dataset.path = path;

  const iconClass = isEmoji ? "sidebar-item-emoji" : "sidebar-item-icon";
  item.innerHTML = `<span class="${iconClass}">${iconHtml}</span><span class="sidebar-item-label">${escapeHtml(label)}</span>`;

  item.addEventListener("click", () => {
    navigate(path).catch(console.error);
  });
  section.appendChild(item);
}

function addSidebarPlaceholder(
  label: string,
  iconHtml: string,
  section: HTMLElement
): void {
  const item = document.createElement("div");
  item.className = "sidebar-item";
  item.innerHTML = `<span class="sidebar-item-emoji">${iconHtml}</span><span class="sidebar-item-label">${escapeHtml(label)}</span>`;
  section.appendChild(item);
}

// ---------------------------------------------------------------------------
// Sidebar SVG icons for Places section
// ---------------------------------------------------------------------------
const PLACES_ICONS: Record<string, string> = {
  home: `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
    <path d="M2 8 L8 2 L14 8"/><path d="M4 7 v7 h3 V10 h2 v4 h3 V7"/>
  </svg>`,
  recent: `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">
    <circle cx="8" cy="8" r="6"/><path d="M8 5 v3 l2.5 2"/>
  </svg>`,
  starred: `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
    <polygon points="8,2 10,6.5 15,7.2 11.5,10.5 12.5,15.5 8,13 3.5,15.5 4.5,10.5 1,7.2 6,6.5"/>
  </svg>`,
  network: `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
    <rect x="1" y="1" width="5" height="4" rx="1"/><rect x="10" y="1" width="5" height="4" rx="1"/>
    <rect x="5" y="11" width="6" height="4" rx="1"/>
    <path d="M8 5 v3 m-4.5-3 v2 l4.5 1 m4.5-3 v2 l-4.5 1"/>
  </svg>`,
  trash: `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
    <path d="M3 5 h10 l-1 9 H4 z"/><path d="M6 5 V3 h4 v2"/><line x1="1" y1="5" x2="15" y2="5"/>
    <line x1="6.5" y1="7" x2="6.5" y2="12"/><line x1="9.5" y1="7" x2="9.5" y2="12"/>
  </svg>`,
};

// ---------------------------------------------------------------------------
// Initialise
// ---------------------------------------------------------------------------
async function init(): Promise<void> {
  listingEl = document.querySelector<HTMLElement>("#listing");
  crumbSegmentsEl = document.querySelector<HTMLElement>("#crumb-segments");
  crumbbarEl = document.querySelector<HTMLElement>("#crumbbar");

  // Create and insert error banner
  const mainEl = document.querySelector<HTMLElement>(".main");
  if (mainEl) {
    errorBannerEl = document.createElement("div");
    errorBannerEl.className = "error-banner";
    errorBannerEl.setAttribute("role", "alert");
    errorBannerEl.setAttribute("aria-live", "polite");

    const dismissBtn = document.createElement("button");
    dismissBtn.className = "error-banner-dismiss";
    dismissBtn.textContent = "×";
    dismissBtn.setAttribute("aria-label", "Dismiss");
    dismissBtn.addEventListener("click", hideErrorBanner);
    errorBannerEl.appendChild(dismissBtn);

    // Insert after headerbar
    const headerbar = mainEl.querySelector(".headerbar");
    if (headerbar && headerbar.parentNode) {
      headerbar.parentNode.insertBefore(errorBannerEl, headerbar.nextSibling);
    } else {
      mainEl.prepend(errorBannerEl);
    }
  }

  btnBack = document.querySelector<HTMLButtonElement>("#btn-back");
  btnForward = document.querySelector<HTMLButtonElement>("#btn-forward");

  // Wire up back / forward / up navigation
  if (btnBack) {
    btnBack.addEventListener("click", () => {
      if (backStack.length === 0) return;
      const prev = backStack.pop()!;
      forwardStack.push(currentPath);
      renderInternal(prev).then(updateNavButtons).catch(console.error);
    });
  }

  if (btnForward) {
    btnForward.addEventListener("click", () => {
      if (forwardStack.length === 0) return;
      const next = forwardStack.pop()!;
      backStack.push(currentPath);
      renderInternal(next).then(updateNavButtons).catch(console.error);
    });
  }

  // Wire up Cmd+L / Ctrl+L for editable path bar
  document.addEventListener("keydown", (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "l") {
      e.preventDefault();
      openPathInput();
      return;
    }

    // Cmd+Shift+. (or Ctrl+Shift+.) to toggle hidden files
    if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === ".") {
      e.preventDefault();
      showHidden = !showHidden;
      renderInternal(currentPath).catch(console.error);
      return;
    }

    // F2 on selected item: start rename
    if (e.key === "F2" && selectedEl) {
      e.preventDefault();
      const path = selectedEl.dataset.path;
      if (path) {
        // Find the entry for this path from the listing
        const entryName = path.split("/").pop() ?? "";
        const isDir = selectedEl.dataset.isDir === "true";
        const fakeEntry: Entry = {
          name: entryName,
          path,
          is_dir: isDir,
          size: 0,
          modified: 0,
          is_hidden: entryName.startsWith("."),
          is_symlink: false,
        };
        startRename(fakeEntry, selectedEl);
      }
      return;
    }
  });

  // Wire up view toggle buttons
  const btnGrid = document.querySelector<HTMLButtonElement>("#btn-view-grid");
  if (btnGrid) {
    btnGrid.addEventListener("click", () => {
      setViewMode("icon");
      renderInternal(currentPath).catch(console.error);
    });
  }

  // View toggle chevron: toggle between icon and list view
  const btnChevron = document.querySelector<HTMLButtonElement>("#btn-view-chevron");
  if (btnChevron) {
    btnChevron.addEventListener("click", () => {
      setViewMode(viewMode === "icon" ? "list" : "icon");
      renderInternal(currentPath).catch(console.error);
    });
  }

  // New folder button
  const btnNewFolder = document.querySelector<HTMLButtonElement>("#btn-new-folder");
  if (btnNewFolder) {
    btnNewFolder.addEventListener("click", () => {
      promptNewFolder();
    });
  }

  // Up button: navigate to parent directory
  // We use the crumb-home icon as an "up/home" shortcut click handler
  const crumbHomeEl = document.querySelector<HTMLElement>("#crumb-home");
  if (crumbHomeEl) {
    crumbHomeEl.style.cursor = "pointer";
    crumbHomeEl.addEventListener("click", () => {
      if (!currentPath) return;
      const parent = currentPath.replace(/\/[^/]+\/?$/, "") || "/";
      if (parent !== currentPath) {
        navigate(parent).catch(console.error);
      }
    });
  }

  // Deselect on clicking the listing background
  listingEl?.addEventListener("click", (e: MouseEvent) => {
    const target = e.target as HTMLElement;
    if (target === listingEl || target.classList.contains("icon-grid") || target.classList.contains("listing-list")) {
      if (selectedEl) {
        selectedEl.classList.remove("selected");
        selectedEl = null;
      }
    }
  });

  const placesSection = document.querySelector<HTMLElement>("#sidebar-places");
  const bookmarksSection = document.querySelector<HTMLElement>("#sidebar-bookmarks");
  if (!placesSection || !bookmarksSection) return;

  // Resolve standard paths
  let home = "";
  try { home = await homeDir(); } catch { home = ""; }

  let documents = "";
  let downloads = "";
  let music = "";
  let pictures = "";
  let videos = "";
  try {
    documents = await documentDir();
    downloads = await downloadDir();
    music = await audioDir();
    pictures = await pictureDir();
    videos = await videoDir();
  } catch {
    documents = home ? `${home}/Documents` : "";
    downloads = home ? `${home}/Downloads` : "";
    music     = home ? `${home}/Music`     : "";
    pictures  = home ? `${home}/Pictures`  : "";
    videos    = home ? `${home}/Videos`    : "";
  }

  if (home) {
    addSidebarItem("Home", PLACES_ICONS.home, home, placesSection);
  }
  addSidebarPlaceholder("Recent",  PLACES_ICONS.recent,  placesSection);
  addSidebarPlaceholder("Starred", PLACES_ICONS.starred, placesSection);
  addSidebarPlaceholder("Network", PLACES_ICONS.network, placesSection);
  addSidebarPlaceholder("Trash",   PLACES_ICONS.trash,   placesSection);

  if (documents) addSidebarItem("Documents", folderIcon("Documents"), documents, bookmarksSection);
  if (music)     addSidebarItem("Music",     folderIcon("Music"),     music,      bookmarksSection);
  if (pictures)  addSidebarItem("Pictures",  folderIcon("Pictures"),  pictures,   bookmarksSection);
  if (videos)    addSidebarItem("Videos",    folderIcon("Videos"),    videos,     bookmarksSection);
  if (downloads) addSidebarItem("Downloads", folderIcon("Downloads"), downloads,  bookmarksSection);

  updateNavButtons();

  if (home) {
    await renderInternal(home);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  init().catch(console.error);
});

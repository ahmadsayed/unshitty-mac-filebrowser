import { homeDir, documentDir, downloadDir, audioDir, pictureDir, videoDir } from "@tauri-apps/api/path";
import { listDir, type Entry } from "./api";
import { folderIcon, fileIcon } from "./icons";

// ---------------------------------------------------------------------------
// Module-level state (Task 4+ navigation will mutate these)
// ---------------------------------------------------------------------------
export let currentPath = "";
export type ViewMode = "list" | "icon";
export let viewMode: ViewMode = "icon";   // Default: grid/icon view

export function setViewMode(mode: ViewMode): void {
  viewMode = mode;
}

// ---------------------------------------------------------------------------
// DOM refs
// ---------------------------------------------------------------------------
let listingEl: HTMLElement | null = null;
let crumbSegmentsEl: HTMLElement | null = null;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Format bytes into a human-readable string (e.g. "4.2 MB"). */
function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/** Format a Unix timestamp into a locale date string. */
function formatDate(unixSeconds: number): string {
  if (unixSeconds === 0) return "—";
  return new Date(unixSeconds * 1000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

// ---------------------------------------------------------------------------
// XSS protection — escape HTML entities in filenames (SECURITY: preserved)
// ---------------------------------------------------------------------------
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// ---------------------------------------------------------------------------
// Sort helper – dirs first, then alphabetically
// ---------------------------------------------------------------------------
function sortEntries(entries: Entry[]): Entry[] {
  return [...entries].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

// ---------------------------------------------------------------------------
// Breadcrumb update
// ---------------------------------------------------------------------------
function updateBreadcrumb(path: string): void {
  if (!crumbSegmentsEl) return;

  // Strip leading slash and split
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
    // SECURITY: escape each path segment before insertion
    frags.push(`<span class="crumb-seg">${escapeHtml(parts[i])}</span>`);
  }
  crumbSegmentsEl.innerHTML = frags.join("");
}

// ---------------------------------------------------------------------------
// Render: list view
// ---------------------------------------------------------------------------
function renderListView(entries: Entry[]): HTMLElement {
  const container = document.createElement("div");
  container.className = "listing-list";

  // Header row
  const header = document.createElement("div");
  header.className = "list-header";
  // The header has 4 cells: icon (empty), Name, Size, Modified
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

    // Icon: SVG from icons.ts — safe fixed template strings
    const iconSvg = entry.is_dir ? folderIcon(entry.name) : fileIcon(entry);

    // Name: SECURITY — escapeHtml applied
    const nameHtml = escapeHtml(entry.name);

    row.innerHTML = `
      <span class="list-cell-icon">${iconSvg}</span>
      <span class="list-cell-name">${nameHtml}</span>
      <span class="list-cell-size">${sizeText}</span>
      <span class="list-cell-modified">${formatDate(entry.modified)}</span>
    `;

    row.addEventListener("dblclick", () => {
      if (entry.is_dir) {
        render(entry.path).catch(console.error);
      }
      // Files: no-op for now (Task 7 will handle open-in-default-app)
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

    // Icon: SVG from icons.ts — safe fixed template strings
    const iconSvg = entry.is_dir ? folderIcon(entry.name) : fileIcon(entry);

    // Name: SECURITY — escapeHtml applied
    const nameHtml = escapeHtml(entry.name);

    item.innerHTML = `
      <span class="icon-item-icon">${iconSvg}</span>
      <span class="icon-item-name">${nameHtml}</span>
    `;

    item.addEventListener("dblclick", () => {
      if (entry.is_dir) {
        render(entry.path).catch(console.error);
      }
    });

    grid.appendChild(item);
  }
  return grid;
}

// ---------------------------------------------------------------------------
// Core render function (called by this module and future tasks)
// ---------------------------------------------------------------------------
export async function render(path: string): Promise<void> {
  if (!listingEl) return;

  currentPath = path;

  // Update breadcrumb
  updateBreadcrumb(path);

  // Mark the matching sidebar item active
  document.querySelectorAll<HTMLElement>(".sidebar-item").forEach((item) => {
    item.classList.toggle("active", item.dataset.path === path);
  });

  listingEl.innerHTML = "";

  let entries: Entry[];
  try {
    entries = await listDir(path, false);
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

// ---------------------------------------------------------------------------
// Sidebar: add a clickable place with an SVG icon
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
  // SECURITY: label comes from our own code, iconHtml is our own SVG template — both safe
  item.innerHTML = `<span class="${iconClass}">${iconHtml}</span><span class="sidebar-item-label">${escapeHtml(label)}</span>`;

  item.addEventListener("click", () => {
    render(path).catch(console.error);
  });
  section.appendChild(item);
}

/** Add a non-navigable placeholder sidebar item (no path). */
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

  const placesSection = document.querySelector<HTMLElement>("#sidebar-places");
  const bookmarksSection = document.querySelector<HTMLElement>("#sidebar-bookmarks");
  if (!placesSection || !bookmarksSection) return;

  // Wire up view toggle buttons
  const btnGrid = document.querySelector<HTMLButtonElement>("#btn-view-grid");
  if (btnGrid) {
    btnGrid.addEventListener("click", () => {
      setViewMode("icon");
      render(currentPath).catch(console.error);
    });
  }

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
    // Fallback: build paths from home dir
    documents = home ? `${home}/Documents` : "";
    downloads = home ? `${home}/Downloads` : "";
    music     = home ? `${home}/Music`     : "";
    pictures  = home ? `${home}/Pictures`  : "";
    videos    = home ? `${home}/Videos`    : "";
  }

  // ----- Places section: Home + non-navigable placeholders -----
  if (home) {
    addSidebarItem("Home", PLACES_ICONS.home, home, placesSection);
  }
  addSidebarPlaceholder("Recent",  PLACES_ICONS.recent,  placesSection);
  addSidebarPlaceholder("Starred", PLACES_ICONS.starred, placesSection);
  addSidebarPlaceholder("Network", PLACES_ICONS.network, placesSection);
  addSidebarPlaceholder("Trash",   PLACES_ICONS.trash,   placesSection);

  // ----- Bookmarks section: navigable folder shortcuts -----
  if (documents) addSidebarItem("Documents", folderIcon("Documents"), documents, bookmarksSection);
  if (music)     addSidebarItem("Music",     folderIcon("Music"),     music,      bookmarksSection);
  if (pictures)  addSidebarItem("Pictures",  folderIcon("Pictures"),  pictures,   bookmarksSection);
  if (videos)    addSidebarItem("Videos",    folderIcon("Videos"),    videos,     bookmarksSection);
  if (downloads) addSidebarItem("Downloads", folderIcon("Downloads"), downloads,  bookmarksSection);

  // Render the home directory on load
  if (home) {
    await render(home);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  init().catch(console.error);
});

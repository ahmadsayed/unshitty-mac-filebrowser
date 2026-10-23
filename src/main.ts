import { homeDir, desktopDir, documentDir, downloadDir } from "@tauri-apps/api/path";
import { listDir, type Entry } from "./api";

// ---------------------------------------------------------------------------
// Module-level state (Task 4+ navigation will mutate these)
// ---------------------------------------------------------------------------
export let currentPath = "";
export type ViewMode = "list" | "icon";
export let viewMode: ViewMode = "list";

export function setViewMode(mode: ViewMode): void {
  viewMode = mode;
}

// ---------------------------------------------------------------------------
// DOM refs
// ---------------------------------------------------------------------------
let listingEl: HTMLElement | null = null;
let pathLabelEl: HTMLElement | null = null;

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

/** Determine the icon character for a given entry. */
function entryIcon(entry: Entry): string {
  if (entry.is_symlink) return "🔗";
  if (entry.is_dir) return "📁";
  // Basic file-type icons by extension
  const ext = entry.name.split(".").pop()?.toLowerCase() ?? "";
  if (["png", "jpg", "jpeg", "gif", "svg", "webp", "ico"].includes(ext)) return "🖼";
  if (["mp4", "mkv", "mov", "avi", "webm"].includes(ext)) return "🎬";
  if (["mp3", "wav", "flac", "ogg", "aac"].includes(ext)) return "🎵";
  if (["pdf"].includes(ext)) return "📄";
  if (["zip", "tar", "gz", "bz2", "xz", "7z", "rar"].includes(ext)) return "📦";
  return "📄";
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
// Render: list view
// ---------------------------------------------------------------------------
function renderListView(entries: Entry[]): HTMLElement {
  const table = document.createElement("table");
  table.className = "listing-table";

  const thead = document.createElement("thead");
  thead.innerHTML = `
    <tr>
      <th class="col-icon"></th>
      <th class="col-name">Name</th>
      <th class="col-size">Size</th>
      <th class="col-modified">Modified</th>
    </tr>
  `;
  table.appendChild(thead);

  const tbody = document.createElement("tbody");
  for (const entry of entries) {
    const tr = document.createElement("tr");
    tr.className = "entry-row";
    if (entry.is_dir) tr.dataset.isDir = "true";
    tr.dataset.path = entry.path;

    const sizeText = entry.is_dir ? "—" : formatSize(entry.size);

    tr.innerHTML = `
      <td class="col-icon">${entryIcon(entry)}</td>
      <td class="col-name">${escapeHtml(entry.name)}</td>
      <td class="col-size">${sizeText}</td>
      <td class="col-modified">${formatDate(entry.modified)}</td>
    `;

    tr.addEventListener("dblclick", () => {
      if (entry.is_dir) {
        render(entry.path).catch(console.error);
      }
      // Files: no-op for now (Task 7 will handle open-in-default-app)
    });

    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  return table;
}

// ---------------------------------------------------------------------------
// Render: icon view
// ---------------------------------------------------------------------------
function renderIconView(entries: Entry[]): HTMLElement {
  const grid = document.createElement("div");
  grid.className = "icon-grid";

  for (const entry of entries) {
    const item = document.createElement("div");
    item.className = "icon-item";
    item.dataset.path = entry.path;
    if (entry.is_dir) item.dataset.isDir = "true";

    item.innerHTML = `
      <span class="icon-item-icon">${entryIcon(entry)}</span>
      <span class="icon-item-name">${escapeHtml(entry.name)}</span>
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
// XSS protection – escape HTML entities in filenames
// ---------------------------------------------------------------------------
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// ---------------------------------------------------------------------------
// Core render function (called by this module and future tasks)
// ---------------------------------------------------------------------------
export async function render(path: string): Promise<void> {
  if (!listingEl) return;

  currentPath = path;

  // Update path label in toolbar
  if (pathLabelEl) {
    pathLabelEl.textContent = path;
  }

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
// Sidebar place link helper
// ---------------------------------------------------------------------------
function addSidebarPlace(
  label: string,
  iconChar: string,
  path: string,
  sidebarList: HTMLElement
): void {
  const item = document.createElement("div");
  item.className = "sidebar-item";
  item.dataset.path = path;
  item.innerHTML = `<span class="sidebar-icon">${iconChar}</span><span class="sidebar-label">${label}</span>`;
  item.addEventListener("click", () => {
    render(path).catch(console.error);
  });
  sidebarList.appendChild(item);
}

// ---------------------------------------------------------------------------
// Initialise
// ---------------------------------------------------------------------------
async function init(): Promise<void> {
  listingEl = document.querySelector<HTMLElement>("#listing");
  pathLabelEl = document.querySelector<HTMLElement>("#path-label");

  const sidebarList = document.querySelector<HTMLElement>("#sidebar-places");
  if (!sidebarList) return;

  // Resolve standard paths
  let home = "";
  try {
    home = await homeDir();
  } catch {
    home = "";
  }

  let desktop = "";
  let documents = "";
  let downloads = "";
  try {
    desktop = await desktopDir();
    documents = await documentDir();
    downloads = await downloadDir();
  } catch {
    // Fallback: build paths from home dir if specific API calls fail
    desktop = home ? `${home}/Desktop` : "";
    documents = home ? `${home}/Documents` : "";
    downloads = home ? `${home}/Downloads` : "";
  }

  if (home) addSidebarPlace("Home", "🏠", home, sidebarList);
  if (desktop) addSidebarPlace("Desktop", "🖥", desktop, sidebarList);
  if (documents) addSidebarPlace("Documents", "📁", documents, sidebarList);
  if (downloads) addSidebarPlace("Downloads", "⬇", downloads, sidebarList);

  // Render the home directory on load
  if (home) {
    await render(home);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  init().catch(console.error);
});

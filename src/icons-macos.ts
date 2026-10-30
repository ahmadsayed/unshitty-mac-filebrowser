import type { FileKind, FolderKind } from "./icons";

// ---------------------------------------------------------------------------
// The macOS theme's icons: original drawings in the spirit of macOS 26
// (Tahoe), made for this project. No Apple artwork, icon files or SF Symbols
// are copied, traced or extracted. They use the same symbol ids as the GNOME
// set (icons.ts), so the views' markup is the same in both themes; symbolic
// icons not redrawn here keep their GNOME shape.
// ---------------------------------------------------------------------------

/** Thin, rounded line icons for the sidebar, toolbar and folder glyphs. */
const LINE = `fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"`;

export const MACOS_SYMBOLIC: Record<string, string> = {
  "go-previous": `<path d="M10 3 5 8l5 5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>`,
  "go-next": `<path d="m6 3 5 5-5 5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>`,
  "pan-down": `<path d="m4.5 6.5 3.5 3.5 3.5-3.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>`,
  "pan-end": `<path d="m6.5 4.5 3.5 3.5-3.5 3.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>`,
  "system-search": `<circle cx="6.75" cy="6.75" r="4.75" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="m10.4 10.4 3.85 3.85" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/>`,
  "open-menu": `<path d="M2.5 4h11M2.5 8h11M2.5 12h11" ${LINE} stroke-width="1.5"/>`,
  // A horizontal ellipsis in a circle, as Mac toolbars draw "more".
  "view-more": `<circle cx="8" cy="8" r="6.5" ${LINE}/><circle cx="5" cy="8" r="1" fill="currentColor"/><circle cx="8" cy="8" r="1" fill="currentColor"/><circle cx="11" cy="8" r="1" fill="currentColor"/>`,
  "view-grid": `<rect x="2" y="2" width="5" height="5" rx="1.25" ${LINE}/><rect x="9" y="2" width="5" height="5" rx="1.25" ${LINE}/><rect x="2" y="9" width="5" height="5" rx="1.25" ${LINE}/><rect x="9" y="9" width="5" height="5" rx="1.25" ${LINE}/>`,
  "view-list": `<circle cx="2.75" cy="4" r="1" fill="currentColor"/><circle cx="2.75" cy="8" r="1" fill="currentColor"/><circle cx="2.75" cy="12" r="1" fill="currentColor"/><path d="M5.75 4h8M5.75 8h8M5.75 12h8" ${LINE} stroke-width="1.4"/>`,
  "folder-new": `<path d="M1.75 4.25a1.5 1.5 0 0 1 1.5-1.5h3l1.5 1.5h5a1.5 1.5 0 0 1 1.5 1.5v6.25a1.5 1.5 0 0 1-1.5 1.5h-9.5a1.5 1.5 0 0 1-1.5-1.5z" ${LINE}/><path d="M8 6.75v4M6 8.75h4" ${LINE}/>`,
  "folder": `<path d="M1.75 4.25a1.5 1.5 0 0 1 1.5-1.5h3l1.5 1.5h5a1.5 1.5 0 0 1 1.5 1.5v6.25a1.5 1.5 0 0 1-1.5 1.5h-9.5a1.5 1.5 0 0 1-1.5-1.5z" ${LINE}/><path d="M1.75 6.25h12.5" ${LINE}/>`,
  "user-home": `<path d="M2 7.5 8 2.5l6 5" ${LINE}/><path d="M3.75 6.25v7.25h3V10h2.5v3.5h3V6.25" ${LINE}/>`,
  "document-open-recent": `<circle cx="8" cy="8" r="6.25" ${LINE}/><path d="M8 4.5V8l2.5 1.5" ${LINE}/>`,
  "non-starred": `<path d="m8 1.75 1.9 3.95 4.35.55-3.2 3 .82 4.3L8 11.45l-3.87 2.1.82-4.3-3.2-3 4.35-.55z" ${LINE} stroke-width="1.2"/>`,
  "starred": `<path d="m8 1.75 1.9 3.95 4.35.55-3.2 3 .82 4.3L8 11.45l-3.87 2.1.82-4.3-3.2-3 4.35-.55z" fill="currentColor" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/>`,
  // A globe: Mac sidebars show Network as one.
  "network-workgroup": `<circle cx="8" cy="8" r="6.25" ${LINE} stroke-width="1.2"/><ellipse cx="8" cy="8" rx="2.75" ry="6.25" ${LINE} stroke-width="1.2"/><path d="M1.75 8h12.5M2.75 5h10.5M2.75 11h10.5" ${LINE} stroke-width="1.1"/>`,
  "user-trash": `<path d="M2.5 4h11M6 4V2.75h4V4" ${LINE}/><path d="m3.75 4 .7 9.15a1 1 0 0 0 1 .93h5.1a1 1 0 0 0 1-.93L12.25 4M6.5 6.75v4.5M9.5 6.75v4.5" ${LINE}/>`,
  "folder-documents": `<path d="M4 1.75h5l3.25 3.25v8.25a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V2.75a1 1 0 0 1 1-1z" ${LINE}/><path d="M9 1.75V5h3.25M5.25 8h5.5M5.25 10.5h5.5M5.25 13h3" ${LINE} stroke-width="1.1"/>`,
  "folder-music": `<path d="M6.5 12.25V3.75l6.75-1.75v8.5" ${LINE}/><ellipse cx="4.6" cy="12.3" rx="1.9" ry="1.6" fill="currentColor"/><ellipse cx="11.35" cy="10.6" rx="1.9" ry="1.6" fill="currentColor"/>`,
  "folder-pictures": `<rect x="1.75" y="2.75" width="12.5" height="10.5" rx="2" ${LINE}/><path d="m2.25 12 3.75-4 2.75 2.75 1.75-1.75 3.25 3.25" ${LINE}/><circle cx="10.5" cy="5.75" r="1.25" ${LINE} stroke-width="1.1"/>`,
  "folder-videos": `<rect x="1.75" y="2.75" width="12.5" height="10.5" rx="1.75" ${LINE}/><path d="M4.75 2.75v10.5M11.25 2.75v10.5M1.75 6.25h3M1.75 9.75h3M11.25 6.25h3M11.25 9.75h3" ${LINE} stroke-width="1.1"/>`,
  "folder-download": `<circle cx="8" cy="8" r="6.25" ${LINE}/><path d="M8 4.75v6M5.5 8.5 8 11l2.5-2.5" ${LINE}/>`,
  "user-desktop": `<rect x="1.75" y="2.25" width="12.5" height="8.75" rx="1.25" ${LINE}/><path d="M8 11v2.75M5.75 13.75h4.5" ${LINE}/>`,
  "folder-publicshare": `<path d="M5.25 6.25H4.5a1 1 0 0 0-1 1v5.5a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1v-5.5a1 1 0 0 0-1-1h-.75" ${LINE}/><path d="M8 1.75v7.5M5.75 4 8 1.75 10.25 4" ${LINE}/>`,
  "folder-templates": `<rect x="4.75" y="1.75" width="8.5" height="10" rx="1.5" ${LINE}/><path d="M2.75 4.75v8.5a1 1 0 0 0 1 1h6.5" ${LINE}/>`,
  // An alias arrow: thin and curved, for the small badge on links.
  "emblem-symbolic-link": `<path d="M4.5 12.5V9.75A3.25 3.25 0 0 1 7.75 6.5h4.5" ${LINE} stroke-width="1.8"/><path d="m9.75 3.75 2.75 2.75-2.75 2.75" ${LINE} stroke-width="1.8"/>`,
  "drive-harddisk": `<rect x="1.75" y="4.5" width="12.5" height="7" rx="2" ${LINE}/><path d="M4.25 9h3" ${LINE} stroke-width="1.1"/><circle cx="11.5" cy="8" r=".9" fill="currentColor"/>`,
};

// ---------------------------------------------------------------------------
// Folders: a back panel with a tab on the left, a lighter front panel, and
// the special folder's line glyph pressed into the front in a deeper blue.
// ---------------------------------------------------------------------------

/** Gradients the macOS drawings use, installed with the sprite. */
export const MACOS_DEFS = `
  <linearGradient id="fcm-folder-front" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#7cc6f7"/>
    <stop offset="1" stop-color="#57aef0"/>
  </linearGradient>
  <linearGradient id="fcm-folder-back" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#4fa8ec"/>
    <stop offset="1" stop-color="#2f8de0"/>
  </linearGradient>
  <linearGradient id="fcm-app" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#6ac9fb"/>
    <stop offset="1" stop-color="#1d7cf2"/>
  </linearGradient>
  <linearGradient id="fcm-drive" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#f2f2f5"/>
    <stop offset="1" stop-color="#cfd0d5"/>
  </linearGradient>`;

const FOLDER_BACK = `M6 13.5a3 3 0 0 1 3-3h13.6a3 3 0 0 1 2.3 1.08L27.3 14.5H55a3 3 0 0 1 3 3V52a3 3 0 0 1-3 3H9a3 3 0 0 1-3-3z`;

export function macFolderSymbol(kind: FolderKind, glyphs: Record<string, string>, glyphName: string | null): string {
  const glyph = glyphName
    ? `<g transform="translate(21 25.5) scale(1.375)" color="#1f6fbd" opacity=".55">${glyphs[glyphName]}</g>`
    : "";
  return `<symbol id="fc-folder-${kind}" viewBox="0 0 64 64">
    <path d="${FOLDER_BACK}" fill="url(#fcm-folder-back)"/>
    <rect x="6" y="19.5" width="52" height="35.5" rx="3" fill="url(#fcm-folder-front)"/>
    <rect x="7" y="19.5" width="50" height="1" rx=".5" fill="#ffffff" opacity=".6"/>
    <rect x="7" y="54" width="50" height="1" rx=".5" fill="#2a86d6" opacity=".45"/>
    ${glyph}
  </symbol>`;
}

/** The deeper folder silhouette behind a folder that keeps versions. */
export function macFolderBackLayer(): string {
  return `<symbol id="fc-folder-back" viewBox="0 0 64 64">
    <path d="${FOLDER_BACK}" fill="#2178c9"/>
    <rect x="6" y="19.5" width="52" height="35.5" rx="3" fill="#3f98e3"/>
    <rect x="7" y="19.5" width="50" height="1" rx=".5" fill="#ffffff" opacity=".3"/>
  </symbol>`;
}

// ---------------------------------------------------------------------------
// Files: a tall white page with a turned-down corner, a hairline edge and a
// soft shadow, with a glyph in the type's colour.
// ---------------------------------------------------------------------------

const PAGE_SHAPE = `M15.5 4h22l13.5 13.5v39a2.5 2.5 0 0 1-2.5 2.5h-33a2.5 2.5 0 0 1-2.5-2.5v-50A2.5 2.5 0 0 1 15.5 4z`;
const PAGE = `
  <path d="${PAGE_SHAPE}" transform="translate(0 1)" fill="#000000" opacity=".12"/>
  <path d="${PAGE_SHAPE}" fill="#ffffff" stroke="#d1d1d6" stroke-width=".75"/>
  <path d="M37.5 4v11a2.5 2.5 0 0 0 2.5 2.5h11z" fill="#e5e5ea" stroke="#d1d1d6" stroke-width=".75" stroke-linejoin="round"/>`;

const MAC_FILE_GLYPHS: Record<FileKind, string> = {
  text: `<path d="M20 26h24M20 31.5h24M20 37h24M20 42.5h24M20 48h15" stroke="#aeaeb2" stroke-width="2" stroke-linecap="round"/>`,
  code: `<path d="m26.5 30-6 6 6 6M37.5 30l6 6-6 6M34 27l-4 18" fill="none" stroke="#5856d6" stroke-width="2.75" stroke-linecap="round" stroke-linejoin="round"/>`,
  audio: `<path d="M28.5 45V28l13-3.25v17" fill="none" stroke="#ff375f" stroke-width="2.75" stroke-linejoin="round"/><ellipse cx="25.25" cy="45" rx="4.25" ry="3.5" fill="#ff375f"/><ellipse cx="38.25" cy="41.75" rx="4.25" ry="3.5" fill="#ff375f"/>`,
  video: `<rect x="19" y="27" width="26" height="19" rx="4" fill="#af52de"/><path d="m29.5 31.75 7 4.75-7 4.75z" fill="#ffffff"/>`,
  pdf: `<rect x="19" y="24" width="26" height="8" rx="2" fill="#ff3b30"/><path d="M20 37h24M20 42.5h24M20 48h15" stroke="#aeaeb2" stroke-width="2" stroke-linecap="round"/>`,
  document: `<path d="M20 26h24" stroke="#007aff" stroke-width="3.25" stroke-linecap="round"/><path d="M20 32.5h24M20 38h24M20 43.5h24M20 49h15" stroke="#aeaeb2" stroke-width="2" stroke-linecap="round"/>`,
  spreadsheet: `<rect x="19" y="25" width="26" height="23" rx="2.5" fill="#e3f7e8" stroke="#34c759" stroke-width="1.75"/><path d="M19 32.5h26M19 40h26M28 25v23M36.5 25v23" stroke="#34c759" stroke-width="1.5"/>`,
  presentation: `<rect x="18" y="26" width="28" height="19" rx="2.5" fill="#ff9500"/><path d="M24.5 40.5v-4M30.5 40.5v-8M36.5 40.5v-6" stroke="#ffffff" stroke-width="2.75" stroke-linecap="round"/>`,
  executable: "",
  image: "",
  archive: "",
  disk: "",
};

export function macFileSymbol(kind: FileKind): string {
  if (kind === "image") {
    return `<symbol id="fc-file-image" viewBox="0 0 64 64">
      <rect x="6" y="12" width="52" height="41" rx="5" fill="#000000" opacity=".12"/>
      <rect x="6" y="11" width="52" height="41" rx="5" fill="#ffffff" stroke="#d1d1d6" stroke-width=".75"/>
      <rect x="10" y="15" width="44" height="33" rx="2.5" fill="#64d2ff"/>
      <path d="M10 41 21.5 29.5l9.5 9 6.5-5.5L54 44.5V45.5a2.5 2.5 0 0 1-2.5 2.5h-39A2.5 2.5 0 0 1 10 45.5z" fill="#30d158"/>
      <path d="M10 44.5 21.5 34l9.5 8.5 6.5-5L54 47.5a2.5 2.5 0 0 1-2.5.5h-39A2.5 2.5 0 0 1 10 45.5z" fill="#28b14c"/>
      <circle cx="42" cy="23.5" r="4.25" fill="#ffd60a"/>
    </symbol>`;
  }
  if (kind === "archive") {
    // A page with a zipper down its middle.
    const teeth = [0, 1, 2, 3, 4, 5]
      .map((i) => `<rect x="${i % 2 ? 32 : 29}" y="${6 + i * 4}" width="3" height="2.5" rx=".6" fill="#8e8e93"/>`)
      .join("");
    return `<symbol id="fc-file-archive" viewBox="0 0 64 64">${PAGE}${teeth}
      <rect x="28" y="31" width="8" height="11" rx="2" fill="#8e8e93"/>
      <rect x="30" y="33.5" width="4" height="6" rx="1" fill="#ffffff"/>
    </symbol>`;
  }
  if (kind === "disk") {
    // A silver drive with a slot and a status light.
    return `<symbol id="fc-file-disk" viewBox="0 0 64 64">
      <rect x="7" y="19" width="50" height="29" rx="7" fill="#000000" opacity=".12"/>
      <rect x="7" y="18" width="50" height="29" rx="7" fill="url(#fcm-drive)" stroke="#b9bac0" stroke-width=".75"/>
      <rect x="15" y="35" width="22" height="3" rx="1.5" fill="#8e8e93"/>
      <circle cx="47" cy="36.5" r="2.25" fill="#34c759"/>
      <rect x="9" y="19.5" width="46" height="1" rx=".5" fill="#ffffff" opacity=".9"/>
    </symbol>`;
  }
  if (kind === "executable") {
    // A generic app tile: a rounded square with a grid of four.
    return `<symbol id="fc-file-executable" viewBox="0 0 64 64">
      <rect x="8" y="9" width="48" height="48" rx="11" fill="#000000" opacity=".14"/>
      <rect x="8" y="8" width="48" height="48" rx="11" fill="url(#fcm-app)"/>
      <rect x="19" y="19" width="11" height="11" rx="3" fill="#ffffff"/>
      <rect x="34" y="19" width="11" height="11" rx="3" fill="#ffffff" opacity=".85"/>
      <rect x="19" y="34" width="11" height="11" rx="3" fill="#ffffff" opacity=".85"/>
      <rect x="34" y="34" width="11" height="11" rx="3" fill="#ffffff" opacity=".7"/>
    </symbol>`;
  }
  return `<symbol id="fc-file-${kind}" viewBox="0 0 64 64">${PAGE}${MAC_FILE_GLYPHS[kind]}</symbol>`;
}

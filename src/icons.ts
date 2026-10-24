import type { Entry } from "./api";

// ---------------------------------------------------------------------------
// Icons, modelled on the Adwaita icon theme used by GNOME 50.
//
// Every icon is a <symbol> in one sprite injected at startup; views reference
// them with <use>, so a 10k-entry folder doesn't carry 10k copies of the SVG.
//   - Symbolic icons (16×16, monochrome, take `currentColor`): sym-*
//   - Full-colour icons (64×64) for folders and file types: fc-*
// ---------------------------------------------------------------------------

/** Symbolic glyphs, 16×16, drawn in currentColor. */
const SYMBOLIC: Record<string, string> = {
  "go-previous": `<path d="M10.5 2.5 5 8l5.5 5.5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>`,
  "go-next": `<path d="M5.5 2.5 11 8l-5.5 5.5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>`,
  "pan-down": `<path d="m4 6 4 4 4-4" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>`,
  "pan-end": `<path d="m6 4 4 4-4 4" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>`,
  "system-search": `<circle cx="6.75" cy="6.75" r="4.5" fill="none" stroke="currentColor" stroke-width="2"/><path d="m10.25 10.25 4 4" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>`,
  "open-menu": `<rect x="1.5" y="2.5" width="13" height="2" rx="1" fill="currentColor"/><rect x="1.5" y="7" width="13" height="2" rx="1" fill="currentColor"/><rect x="1.5" y="11.5" width="13" height="2" rx="1" fill="currentColor"/>`,
  "view-more": `<circle cx="8" cy="3" r="1.5" fill="currentColor"/><circle cx="8" cy="8" r="1.5" fill="currentColor"/><circle cx="8" cy="13" r="1.5" fill="currentColor"/>`,
  "view-grid": `<rect x="1.5" y="1.5" width="5.5" height="5.5" rx="1.5" fill="currentColor"/><rect x="9" y="1.5" width="5.5" height="5.5" rx="1.5" fill="currentColor"/><rect x="1.5" y="9" width="5.5" height="5.5" rx="1.5" fill="currentColor"/><rect x="9" y="9" width="5.5" height="5.5" rx="1.5" fill="currentColor"/>`,
  "view-list": `<rect x="1" y="2" width="3" height="3" rx="1" fill="currentColor"/><rect x="5.5" y="2.5" width="9.5" height="2" rx="1" fill="currentColor"/><rect x="1" y="6.5" width="3" height="3" rx="1" fill="currentColor"/><rect x="5.5" y="7" width="9.5" height="2" rx="1" fill="currentColor"/><rect x="1" y="11" width="3" height="3" rx="1" fill="currentColor"/><rect x="5.5" y="11.5" width="9.5" height="2" rx="1" fill="currentColor"/>`,
  "folder-new": `<path d="M2.75 3.25h3.5l1.5 1.5h5.5a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1H2.75a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/><path d="M8 7v4.5M5.75 9.25h4.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>`,
  "folder": `<path d="M1.5 3.5a1.5 1.5 0 0 1 1.5-1.5h3.4l1.6 1.6H13a1.5 1.5 0 0 1 1.5 1.5v7.4A1.5 1.5 0 0 1 13 14H3a1.5 1.5 0 0 1-1.5-1.5z" fill="currentColor"/>`,
  "user-home": `<path d="M8 1.25.75 7.5H2.5V14a1 1 0 0 0 1 1H6.5v-4.5h3V15h3a1 1 0 0 0 1-1V7.5h1.75z" fill="currentColor"/>`,
  "document-open-recent": `<circle cx="8" cy="8" r="6.25" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M8 4.25V8l2.75 1.75" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>`,
  "non-starred": `<path d="m8 1.25 2.06 4.3 4.69.6-3.44 3.27.87 4.66L8 11.8l-4.18 2.28.87-4.66L1.25 6.15l4.69-.6z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/>`,
  "starred": `<path d="m8 1.25 2.06 4.3 4.69.6-3.44 3.27.87 4.66L8 11.8l-4.18 2.28.87-4.66L1.25 6.15l4.69-.6z" fill="currentColor"/>`,
  "network-workgroup": `<rect x="5.5" y="1" width="5" height="4" rx="1" fill="currentColor"/><rect x="1" y="11" width="5" height="4" rx="1" fill="currentColor"/><rect x="10" y="11" width="5" height="4" rx="1" fill="currentColor"/><path d="M8 5v3M3.5 11V8.5h9V11" fill="none" stroke="currentColor" stroke-width="1.5"/>`,
  "user-trash": `<path d="M6 1.5h4a1 1 0 0 1 1 1V3h3a.75.75 0 0 1 0 1.5H2A.75.75 0 0 1 2 3h3v-.5a1 1 0 0 1 1-1z" fill="currentColor"/><path d="M3 5.5h10l-.85 8.6a1.5 1.5 0 0 1-1.5 1.4H5.35a1.5 1.5 0 0 1-1.5-1.4z" fill="currentColor"/>`,
  "folder-documents": `<path d="M3.5 1h6l3.5 3.5V14a1 1 0 0 1-1 1H3.5a1 1 0 0 1-1-1V2a1 1 0 0 1 1-1z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/><path d="M5 7.5h6M5 10h6M5 12.5h4" stroke="currentColor" stroke-width="1.25" stroke-linecap="round"/>`,
  "folder-music": `<path d="M6 12.25V3.5l8-2v9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/><ellipse cx="4" cy="12.5" rx="2.25" ry="2" fill="currentColor"/><ellipse cx="12" cy="10.75" rx="2.25" ry="2" fill="currentColor"/>`,
  "folder-pictures": `<rect x="1.25" y="2.25" width="13.5" height="11.5" rx="2" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="m2.5 12.5 3.5-4 2.5 2.5 2-2 3 3.5z" fill="currentColor"/><circle cx="10.5" cy="5.75" r="1.5" fill="currentColor"/>`,
  "folder-videos": `<rect x="1" y="3" width="10" height="10" rx="2" fill="currentColor"/><path d="m12 6.5 3-2v7l-3-2z" fill="currentColor"/>`,
  "folder-download": `<path d="M8 1.5v8M4.75 6.5 8 9.75l3.25-3.25" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/><path d="M1.75 10.5v2.25a1.5 1.5 0 0 0 1.5 1.5h9.5a1.5 1.5 0 0 0 1.5-1.5V10.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>`,
  "user-desktop": `<rect x="1" y="2" width="14" height="9.5" rx="1.5" fill="currentColor"/><rect x="4.5" y="13" width="7" height="1.5" rx=".75" fill="currentColor"/>`,
  "folder-publicshare": `<circle cx="4" cy="8" r="2.25" fill="currentColor"/><circle cx="12" cy="3.5" r="2.25" fill="currentColor"/><circle cx="12" cy="12.5" r="2.25" fill="currentColor"/><path d="m4 8 8-4.5M4 8l8 4.5" stroke="currentColor" stroke-width="1.5"/>`,
  "folder-templates": `<rect x="2" y="1.5" width="12" height="13" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-dasharray="2.5 1.5"/>`,
  "drive-harddisk": `<path d="M3.2 2.5h9.6l2.2 7v3a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 1 12.5v-3z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/><path d="M1.5 9.5h13" stroke="currentColor" stroke-width="1.5"/><circle cx="11.75" cy="11.75" r="1" fill="currentColor"/>`,
  "media-eject": `<path d="M8 2.5 13.5 9h-11z" fill="currentColor"/><rect x="2.5" y="11" width="11" height="2.5" rx="1" fill="currentColor"/>`,
  "emblem-symbolic-link": `<path d="M4 12V9.5a3 3 0 0 1 3-3h5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><path d="m9.5 3.5 3 3-3 3" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>`,
  "object-select": `<path d="m3 8.5 3.25 3.25L13 5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>`,
  "window-close": `<path d="m4.5 4.5 7 7m0-7-7 7" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>`,
  "dialog-error": `<circle cx="8" cy="8" r="6.5" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M8 4.5v4.25" stroke="currentColor" stroke-width="1.75" stroke-linecap="round"/><circle cx="8" cy="11.25" r="1" fill="currentColor"/>`,
  "sort-ascending": `<path d="m4.5 10 3.5-4 3.5 4" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>`,
  "sort-descending": `<path d="m4.5 6 3.5 4 3.5-4" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"/>`,
};

// ---------------------------------------------------------------------------
// Full-colour folder (Adwaita 50 style): a darker-blue back panel with a tab,
// a lighter-blue front panel, and an optional symbolic glyph embossed on the
// front for the well-known XDG folders.
// ---------------------------------------------------------------------------

export type FolderKind =
  | "plain"
  | "home"
  | "desktop"
  | "documents"
  | "download"
  | "music"
  | "pictures"
  | "videos"
  | "publicshare"
  | "templates";

const FOLDER_KIND_GLYPH: Record<Exclude<FolderKind, "plain">, string> = {
  home: "user-home",
  desktop: "user-desktop",
  documents: "folder-documents",
  download: "folder-download",
  music: "folder-music",
  pictures: "folder-pictures",
  videos: "folder-videos",
  publicshare: "folder-publicshare",
  templates: "folder-templates",
};

function folderSymbol(kind: FolderKind): string {
  const glyph =
    kind === "plain"
      ? ""
      : `<g transform="translate(23 30.5) scale(1.125)" color="#3584e4" opacity=".85">${SYMBOLIC[FOLDER_KIND_GLYPH[kind]]}</g>`;
  return `<symbol id="fc-folder-${kind}" viewBox="0 0 64 64">
    <path d="M6 13a4 4 0 0 1 4-4h13.2a4 4 0 0 1 2.83 1.17L29 13h25a4 4 0 0 1 4 4v35H6z" fill="#438de6"/>
    <rect x="9" y="15" width="46" height="20" rx="2" fill="#f6f5f4"/>
    <rect x="6" y="19" width="52" height="38" rx="4" fill="url(#fc-folder-front)"/>
    <path d="M6 53h52a4 4 0 0 1-4 4H10a4 4 0 0 1-4-4z" fill="#62a0ea" opacity=".55"/>
    <rect x="6" y="19" width="52" height="1" fill="#ffffff" opacity=".55"/>
    ${glyph}
  </symbol>`;
}

// ---------------------------------------------------------------------------
// Full-colour file icons: a white page with a folded corner plus a coloured
// mime-type glyph, roughly matching Adwaita's mimetype icons.
// ---------------------------------------------------------------------------

export type FileKind =
  | "text"
  | "code"
  | "image"
  | "audio"
  | "video"
  | "archive"
  | "disk"
  | "pdf"
  | "document"
  | "spreadsheet"
  | "presentation"
  | "executable";

const PAGE = `
  <path d="M15 5h23l13 13v39a3 3 0 0 1-3 3H15a3 3 0 0 1-3-3V8a3 3 0 0 1 3-3z" fill="#c0bfbc"/>
  <path d="M15 4h23l13 13v38a3 3 0 0 1-3 3H15a3 3 0 0 1-3-3V7a3 3 0 0 1 3-3z" fill="#ffffff"/>
  <path d="M38 4v10a3 3 0 0 0 3 3h10z" fill="#deddda"/>`;

const FILE_GLYPHS: Record<FileKind, string> = {
  text: `<path d="M20 26h24M20 32h24M20 38h24M20 44h16" stroke="#9a9996" stroke-width="2.5" stroke-linecap="round"/>`,
  code: `<path d="m26 29-6 6 6 6M38 29l6 6-6 6M34 26l-4 18" fill="none" stroke="#5e5c64" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>`,
  audio: `<path d="M29 45V27l12-3v17" fill="none" stroke="#e66100" stroke-width="3" stroke-linejoin="round"/><ellipse cx="26" cy="45" rx="4.5" ry="3.75" fill="#e66100"/><ellipse cx="38" cy="41.5" rx="4.5" ry="3.75" fill="#e66100"/>`,
  video: `<rect x="18" y="26" width="28" height="20" rx="3" fill="#3d3846"/><path d="m29 31 8 5-8 5z" fill="#ffffff"/>`,
  pdf: `<rect x="18" y="24" width="28" height="9" rx="2" fill="#e01b24"/><path d="M20 38h24M20 43h24M20 48h16" stroke="#9a9996" stroke-width="2.5" stroke-linecap="round"/>`,
  document: `<path d="M20 26h24" stroke="#3584e4" stroke-width="3.5" stroke-linecap="round"/><path d="M20 33h24M20 39h24M20 45h16" stroke="#99c1f1" stroke-width="2.5" stroke-linecap="round"/>`,
  spreadsheet: `<rect x="18" y="24" width="28" height="24" rx="2" fill="#57e389"/><path d="M18 32h28M18 40h28M27 24v24M36 24v24" stroke="#26a269" stroke-width="2"/>`,
  presentation: `<rect x="17" y="25" width="30" height="20" rx="2" fill="#ffa348"/><path d="M24 40v-5M30 40v-9M36 40v-7" stroke="#ffffff" stroke-width="3" stroke-linecap="round"/>`,
  executable: `<circle cx="32" cy="36" r="9" fill="none" stroke="#5e5c64" stroke-width="4" stroke-dasharray="4.7 2.4"/><circle cx="32" cy="36" r="4" fill="#5e5c64"/>`,
  image: "",
  archive: "",
  disk: "",
};

function fileSymbol(kind: FileKind): string {
  if (kind === "image") {
    // Adwaita's image icon is a framed photo, not a page.
    return `<symbol id="fc-file-image" viewBox="0 0 64 64">
      <rect x="6" y="12" width="52" height="42" rx="4" fill="#c0bfbc"/>
      <rect x="6" y="11" width="52" height="42" rx="4" fill="#ffffff"/>
      <rect x="10" y="15" width="44" height="34" rx="2" fill="#99c1f1"/>
      <path d="M10 41.5 22 30l10 9 6-5 16 12v1a2 2 0 0 1-2 2H12a2 2 0 0 1-2-2z" fill="#33d17a"/>
      <path d="M10 45 22 34l10 9 6-5 16 10v-1 2a2 2 0 0 1-2 2H12a2 2 0 0 1-2-2z" fill="#26a269"/>
      <circle cx="42" cy="24" r="4.5" fill="#f6d32d"/>
    </symbol>`;
  }
  if (kind === "archive") {
    return `<symbol id="fc-file-archive" viewBox="0 0 64 64">
      <path d="M10 20h44v33a4 4 0 0 1-4 4H14a4 4 0 0 1-4-4z" fill="#b5835a"/>
      <path d="M14 9h36l6 11H8z" fill="#cdab8f"/>
      <rect x="8" y="19" width="48" height="5" fill="#986a44"/>
      <rect x="27" y="9" width="10" height="20" fill="#f6f5f4"/>
      <rect x="27" y="24" width="10" height="5" fill="#deddda"/>
    </symbol>`;
  }
  if (kind === "disk") {
    return `<symbol id="fc-file-disk" viewBox="0 0 64 64">
      <circle cx="32" cy="33" r="25" fill="#c0bfbc"/>
      <circle cx="32" cy="32" r="25" fill="#deddda"/>
      <circle cx="32" cy="32" r="18" fill="#f6f5f4"/>
      <circle cx="32" cy="32" r="7" fill="#9a9996"/>
      <circle cx="32" cy="32" r="3" fill="#ffffff"/>
    </symbol>`;
  }
  return `<symbol id="fc-file-${kind}" viewBox="0 0 64 64">${PAGE}${FILE_GLYPHS[kind]}</symbol>`;
}

const EXT_KIND: Record<string, FileKind> = {};
function mapExt(kind: FileKind, exts: string): void {
  for (const e of exts.split(" ")) EXT_KIND[e] = kind;
}
mapExt("image", "png jpg jpeg gif svg webp ico bmp tif tiff heic heif avif raw cr2 nef psd");
mapExt("video", "mp4 mkv mov avi webm flv m4v wmv mpg mpeg");
mapExt("audio", "mp3 wav flac ogg oga opus aac m4a aiff aif wma");
mapExt("archive", "zip tar gz tgz bz2 xz zst 7z rar jar deb rpm pkg");
mapExt("disk", "dmg iso img");
mapExt("code", "ts tsx js jsx mjs cjs py rs go c cc cpp h hpp java kt swift sh bash zsh fish rb php pl lua toml json yaml yml xml html htm css scss vue sql");
mapExt("pdf", "pdf");
mapExt("document", "doc docx odt rtf pages epub");
mapExt("spreadsheet", "xls xlsx ods csv tsv numbers");
mapExt("presentation", "ppt pptx odp key");
mapExt("text", "txt md markdown log ini conf cfg");
mapExt("executable", "exe bin run appimage command");

export function fileKind(name: string): FileKind {
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return "text";
  return EXT_KIND[name.slice(dot + 1).toLowerCase()] ?? "text";
}

/** Human-readable type, used for sorting by type. */
export function fileTypeLabel(entry: Entry): string {
  return entry.is_dir ? "folder" : fileKind(entry.name);
}

// ---------------------------------------------------------------------------
// Public builders
// ---------------------------------------------------------------------------

/** A symbolic icon that inherits the surrounding text colour. */
export function sym(name: string, cls = ""): string {
  return `<svg class="sym ${cls}" viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-${name}"/></svg>`;
}

/** A macOS .app bundle: a folder that behaves like a file. */
export function isAppBundle(entry: Entry): boolean {
  return entry.is_dir && /\.app$/i.test(entry.name);
}

/** Full-colour icon for a listing entry; `folderKind` comes from the XDG folder map. */
export function entryIcon(entry: Entry, folderKind: FolderKind = "plain"): string {
  // macOS app bundles are folders, but show (and open) as applications.
  const id = isAppBundle(entry)
    ? "fc-file-executable"
    : entry.is_dir
      ? `fc-folder-${folderKind}`
      : `fc-file-${fileKind(entry.name)}`;
  const link = entry.is_symlink
    ? `<span class="emblem">${sym("emblem-symbolic-link")}</span>`
    : "";
  return `<svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#${id}"/></svg>${link}`;
}

/** Inject the icon sprite into the document once, at startup. */
export function installIconSprite(): void {
  const folders: FolderKind[] = ["plain", "home", "desktop", "documents", "download", "music", "pictures", "videos", "publicshare", "templates"];
  const files = Object.keys(FILE_GLYPHS) as FileKind[];
  const symbolic = Object.entries(SYMBOLIC)
    .map(([name, body]) => `<symbol id="sym-${name}" viewBox="0 0 16 16">${body}</symbol>`)
    .join("");

  const sprite = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  sprite.setAttribute("aria-hidden", "true");
  sprite.style.position = "absolute";
  sprite.style.width = "0";
  sprite.style.height = "0";
  sprite.innerHTML = `<defs>
      <linearGradient id="fc-folder-front" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" stop-color="#a4caee"/>
        <stop offset="1" stop-color="#8dbbf0"/>
      </linearGradient>
    </defs>
    ${symbolic}
    ${folders.map(folderSymbol).join("")}
    ${files.map(fileSymbol).join("")}`;
  document.body.prepend(sprite);
}

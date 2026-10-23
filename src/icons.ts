import type { Entry } from "./api";

// ---------------------------------------------------------------------------
// Adwaita-style SVG folder icon
// Darker-blue tab over a lighter-blue rounded face
// ---------------------------------------------------------------------------

/** Symbolic glyphs that sit inside the folder face, keyed by well-known name */
const FOLDER_GLYPHS: Record<string, string> = {
  // Downloads — downward arrow
  downloads: `<path d="M32 20 v16 m0 0 l-6-6 m6 6 l6-6 M22 42 h20" stroke="#fff" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>`,
  // Music — eighth note
  music: `<path d="M26 42 a5 5 0 1 0 10 0 a5 5 0 0 0 -10 0 M36 42 V22 l10-3 v5 l-10 3" stroke="#fff" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" fill="none"/>`,
  // Pictures — mountain + sun (image)
  pictures: `<rect x="20" y="22" width="24" height="20" rx="2.5" stroke="#fff" stroke-width="2.5" fill="none"/><path d="M20 36 l7-7 5 5 4-4 8 8" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" fill="none"/><circle cx="27" cy="28" r="2.5" fill="#fff"/>`,
  // Videos — play triangle inside a rounded rect
  videos: `<rect x="18" y="20" width="28" height="22" rx="3" stroke="#fff" stroke-width="2.5" fill="none"/><path d="M28 27 l10 4 -10 4 z" fill="#fff"/>`,
  // Public — share / network nodes
  public: `<circle cx="32" cy="20" r="3.5" fill="#fff"/><circle cx="20" cy="42" r="3.5" fill="#fff"/><circle cx="44" cy="42" r="3.5" fill="#fff"/><path d="M32 23.5 L20 38.5 M32 23.5 L44 38.5 M20 42 L44 42" stroke="#fff" stroke-width="2" stroke-linecap="round" fill="none"/>`,
  // Templates / Documents — lines on a page
  templates: `<rect x="22" y="18" width="20" height="26" rx="3" stroke="#fff" stroke-width="2.5" fill="none"/><path d="M26 26 h12 M26 31 h12 M26 36 h8" stroke="#fff" stroke-width="2" stroke-linecap="round" fill="none"/>`,
  documents: `<rect x="22" y="18" width="20" height="26" rx="3" stroke="#fff" stroke-width="2.5" fill="none"/><path d="M26 26 h12 M26 31 h12 M26 36 h8" stroke="#fff" stroke-width="2" stroke-linecap="round" fill="none"/>`,
};

/**
 * Build the Adwaita-style folder SVG string.
 * @param name  The display name of the folder (used to pick a symbolic glyph).
 * @returns     A full SVG string (safe to set as innerHTML — all content is fixed templates).
 */
export function folderIcon(name: string): string {
  const key = name.toLowerCase();
  const glyph = FOLDER_GLYPHS[key] ?? "";

  return `<svg viewBox="0 0 64 64" xmlns="http://www.w3.org/2000/svg" class="folder-icon" aria-hidden="true">
  <!-- Folder tab (darker blue) -->
  <path d="M6 20 a5 5 0 0 1 5-5 h14 l5 5 h23 a5 5 0 0 1 5 5 v3 H6 Z" fill="#4a90e2"/>
  <!-- Folder face (lighter blue) -->
  <rect x="6" y="20" width="52" height="36" rx="6" fill="#b8d4f5"/>
  <rect x="6" y="20" width="52" height="36" rx="6" fill="none" stroke="#9bc0ef" stroke-width="1"/>
  <!-- Symbolic glyph -->
  ${glyph}
</svg>`;
}

// ---------------------------------------------------------------------------
// File icon — a generic document, optionally varied by extension
// ---------------------------------------------------------------------------

type FileIconVariant =
  | "image"
  | "video"
  | "audio"
  | "archive"
  | "code"
  | "pdf"
  | "document";

function extensionVariant(ext: string): FileIconVariant {
  if (["png", "jpg", "jpeg", "gif", "svg", "webp", "ico", "bmp", "tiff"].includes(ext)) return "image";
  if (["mp4", "mkv", "mov", "avi", "webm", "flv"].includes(ext)) return "video";
  if (["mp3", "wav", "flac", "ogg", "aac", "m4a"].includes(ext)) return "audio";
  if (["zip", "tar", "gz", "bz2", "xz", "7z", "rar", "dmg", "iso"].includes(ext)) return "archive";
  if (["ts", "tsx", "js", "jsx", "py", "rs", "go", "c", "cpp", "h", "java", "sh", "rb", "toml", "json", "yaml", "yml", "xml", "html", "css"].includes(ext)) return "code";
  if (ext === "pdf") return "pdf";
  return "document";
}

const VARIANT_COLORS: Record<FileIconVariant, string> = {
  image:    "#26a269",
  video:    "#9141ac",
  audio:    "#e5a50a",
  archive:  "#986a44",
  code:     "#1c71d8",
  pdf:      "#c01c28",
  document: "#3d3846",
};

const VARIANT_GLYPHS: Record<FileIconVariant, string> = {
  image:    `<path d="M8 17 h8 m-4-4 v8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/><circle cx="9" cy="15" r="2" fill="currentColor" opacity="0.6"/>`,
  video:    `<polygon points="7,12 15,16 7,20" fill="currentColor" opacity="0.8"/>`,
  audio:    `<path d="M8 14 a4 4 0 0 1 8 0 a4 4 0 0 1 -8 0 M10 17 v4 M14 17 v4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/>`,
  archive:  `<path d="M11 10 v3 m0 2 v3 m0 2 v3" stroke="currentColor" stroke-width="2" stroke-linecap="round" fill="none"/>`,
  code:     `<path d="M9 14 l-3 2 3 2 M15 14 l3 2 -3 2 M13 11 l-2 8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" fill="none"/>`,
  pdf:      `<path d="M7 14 h10 M7 17 h10 M7 20 h7" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/>`,
  document: `<path d="M7 14 h10 M7 17 h10 M7 20 h7" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/>`,
};

/**
 * Build a file icon SVG. Varies by extension; always returns a safe SVG string.
 */
export function fileIcon(entry: Entry): string {
  const ext = entry.name.split(".").pop()?.toLowerCase() ?? "";
  const variant = extensionVariant(ext);
  const color = VARIANT_COLORS[variant];
  const glyph = VARIANT_GLYPHS[variant];

  return `<svg viewBox="0 0 24 28" xmlns="http://www.w3.org/2000/svg" class="file-icon" aria-hidden="true">
  <!-- Page body -->
  <path d="M4 2 h11 l5 5 v19 a2 2 0 0 1 -2 2 H4 a2 2 0 0 1 -2 -2 V4 a2 2 0 0 1 2 -2 z" fill="${color}" opacity="0.12" stroke="${color}" stroke-width="1.2"/>
  <!-- Folded corner -->
  <path d="M15 2 v4 a1 1 0 0 0 1 1 h4" fill="none" stroke="${color}" stroke-width="1.2"/>
  <!-- Variant glyph (color: ${color}) -->
  <g color="${color}" transform="translate(0, 4)">
    ${glyph}
  </g>
</svg>`;
}

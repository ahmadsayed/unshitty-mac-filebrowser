// The download page's helpers (tested in tests/file-page.test.ts): file
// types and sizes, the picture shown in place of the file icon once an
// image has downloaded, and the rows that list several files.

/** Enough types for a phone to offer the right app after saving. */
const TYPES = {
  pdf: "application/pdf", txt: "text/plain", md: "text/markdown", csv: "text/csv", json: "application/json",
  jpg: "image/jpeg", jpeg: "image/jpeg", png: "image/png", gif: "image/gif", webp: "image/webp", heic: "image/heic",
  avif: "image/avif",
  mp4: "video/mp4", mov: "video/quicktime", mp3: "audio/mpeg", m4a: "audio/mp4", wav: "audio/wav",
  zip: "application/zip",
};

export const typeOf = (name) => TYPES[name.split(".").pop().toLowerCase()] ?? "application/octet-stream";

/** Pictures worth previewing. HEIC and AVIF show only where the browser
 * decodes them; SVG is left out, it's a document more than a photo. */
const IMAGES = new Set(["image/jpeg", "image/png", "image/gif", "image/webp", "image/heic", "image/avif"]);

export const isImage = (type) => IMAGES.has(type);

/** Load `url` and, once the browser has decoded it, put it where `icon`
 * is. A picture it can't decode is dropped and the icon stays. */
export function showImage(icon, url, name, className = "preview") {
  const image = document.createElement("img");
  image.className = className;
  image.alt = name;
  image.addEventListener("load", () => icon.replaceWith(image), { once: true });
  image.addEventListener("error", () => image.remove(), { once: true });
  image.src = url;
  return image;
}

/** GNOME-style sizes: "4 bytes", "12.3 kB", "4.5 MB". */
export function formatSize(bytes) {
  if (bytes < 1000) return `${bytes} ${bytes === 1 ? "byte" : "bytes"}`;
  const units = ["kB", "MB", "GB", "TB"];
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

/** A small file icon (fixed markup) for list rows. */
const ROW_ICON =
  `<svg class="row-icon" viewBox="0 0 64 64" aria-hidden="true">` +
  `<path fill="currentColor" opacity=".18" d="M16 6h22l14 14v36a4 4 0 0 1-4 4H16a4 4 0 0 1-4-4V10a4 4 0 0 1 4-4z"/>` +
  `<path fill="currentColor" d="M38 6v10a4 4 0 0 0 4 4h10z"/></svg>`;

/** A row for one downloaded file: its picture (once decoded) or an icon,
 * name, size and Save. `thumb` is the picture being loaded, if any. */
export function fileRow(file, url, save) {
  const row = document.createElement("li");
  row.className = "file-row";
  row.innerHTML = `${ROW_ICON}<div class="row-text"><div class="row-name"></div><div class="row-size dim"></div></div>` +
    `<button class="row-save" type="button">Save</button>`;
  row.querySelector(".row-name").textContent = file.name;
  row.querySelector(".row-size").textContent = formatSize(file.size);
  row.querySelector(".row-save").addEventListener("click", () => save(file));
  const thumb = isImage(file.type) ? showImage(row.querySelector(".row-icon"), url, file.name, "row-thumb") : null;
  return { row, thumb };
}

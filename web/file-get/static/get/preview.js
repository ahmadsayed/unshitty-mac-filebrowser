// File types for the download page, and the picture shown in place of the
// file icon once an image has downloaded (tested in tests/file-page.test.ts).

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
export function showImage(icon, url, name) {
  const image = document.createElement("img");
  image.className = "preview";
  image.alt = name;
  image.addEventListener("load", () => icon.replaceWith(image), { once: true });
  image.addEventListener("error", () => image.remove(), { once: true });
  image.src = url;
  return image;
}

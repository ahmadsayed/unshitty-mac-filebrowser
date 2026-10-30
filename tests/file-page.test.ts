// The phone's download page (web/file-get): file types, and the image
// preview that replaces the file icon once a picture has downloaded.
import { beforeEach, expect, test } from "vitest";
import { isImage, showImage, typeOf } from "../web/file-get/static/get/preview.js";

let icon: Element;

beforeEach(() => {
  document.body.innerHTML = `<main><svg id="icon" class="icon"></svg><h1 id="name">photo.jpg</h1></main>`;
  icon = document.getElementById("icon")!;
});

test("types come from the extension, in any case", () => {
  expect(typeOf("photo.JPG")).toBe("image/jpeg");
  expect(typeOf("scan.heic")).toBe("image/heic");
  expect(typeOf("cover.avif")).toBe("image/avif");
  expect(typeOf("report.pdf")).toBe("application/pdf");
  expect(typeOf("archive.tar.zst")).toBe("application/octet-stream");
  expect(typeOf("README")).toBe("application/octet-stream");
});

test("photos and pictures are previewed; other files, and SVG, are not", () => {
  for (const type of ["image/jpeg", "image/png", "image/gif", "image/webp", "image/heic", "image/avif"]) {
    expect(isImage(type), type).toBe(true);
  }
  for (const type of ["image/svg+xml", "application/pdf", "video/mp4", "application/octet-stream"]) {
    expect(isImage(type), type).toBe(false);
  }
});

test("a picture replaces the icon once the browser has decoded it", () => {
  const image = showImage(icon, "blob:photo", "photo.jpg");
  // Until it loads, the icon stays: an undecodable picture never shows.
  expect(document.getElementById("icon")).toBe(icon);
  expect(image.isConnected).toBe(false);
  image.dispatchEvent(new Event("load"));
  expect(document.getElementById("icon")).toBeNull();
  const shown = document.querySelector("main img.preview") as HTMLImageElement;
  expect(shown).toBe(image);
  expect(shown.getAttribute("src")).toBe("blob:photo");
  expect(shown.alt).toBe("photo.jpg");
  expect(shown.nextElementSibling?.id).toBe("name");
});

test("a picture the browser can't decode leaves the icon", () => {
  const image = showImage(icon, "blob:scan", "scan.heic");
  image.dispatchEvent(new Event("error"));
  expect(document.getElementById("icon")).toBe(icon);
  expect(document.querySelector("img")).toBeNull();
});

// The drag image for dragging out on macOS: a PNG every decoder accepts.
// The drag plugin's macOS code gives up (and can take the app down) on an
// image NSImage refuses, so a corrupt one must never ship again.
import { crc32, inflateSync } from "node:zlib";
import { expect, test } from "vitest";
import { DRAG_IMAGE, leftWindow } from "../src/drag-out";

test("the drag image is a valid PNG: signature, chunk checksums and image data", () => {
  const prefix = "data:image/png;base64,";
  expect(DRAG_IMAGE.startsWith(prefix)).toBe(true);
  const png = Buffer.from(DRAG_IMAGE.slice(prefix.length), "base64");
  expect(png.subarray(0, 8)).toEqual(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]));
  const types: string[] = [];
  const idat: Buffer[] = [];
  for (let at = 8; at < png.length; ) {
    const length = png.readUInt32BE(at);
    const typeAndData = png.subarray(at + 4, at + 8 + length);
    const type = typeAndData.subarray(0, 4).toString("ascii");
    expect(crc32(typeAndData) >>> 0, `${type} CRC`).toBe(png.readUInt32BE(at + 8 + length));
    if (type === "IDAT") idat.push(typeAndData.subarray(4));
    types.push(type);
    at += 12 + length;
  }
  expect(types).toEqual(["IHDR", "IDAT", "IEND"]);
  // inflateSync checks the zlib stream's own checksum too.
  expect(() => inflateSync(Buffer.concat(idat))).not.toThrow();
});

test("a move counts as leaving the window only past one of its edges", () => {
  const at = (clientX: number, clientY: number) => new MouseEvent("mousemove", { clientX, clientY });
  expect(leftWindow(at(10, 10))).toBe(false);
  expect(leftWindow(at(innerWidth - 1, innerHeight - 1))).toBe(false);
  expect(leftWindow(at(-1, 10))).toBe(true);
  expect(leftWindow(at(10, -1))).toBe(true);
  expect(leftWindow(at(innerWidth, 10))).toBe(true);
  expect(leftWindow(at(10, innerHeight + 1))).toBe(true);
});

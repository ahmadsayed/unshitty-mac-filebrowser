// The "has other files" emblem as each theme's sprite is parsed (spec F6):
// the stroke widths the source states are the ones drawn. The HTML parser
// keeps the first of two `stroke-width` attributes, so a width written after
// a shared attribute string would be silently dropped.
import { expect, test } from "vitest";
import { spriteMarkup } from "../src/icons";

function emblemWidths(theme: "gnome" | "macos"): string[] {
  const sprite = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  sprite.innerHTML = spriteMarkup(theme);
  return [...sprite.querySelectorAll("#sym-emblem-files path")].map((path) => path.getAttribute("stroke-width") ?? "");
}

test("the macOS emblem draws its sheets at 1.6 and its fold at 1.3", () => {
  expect(emblemWidths("macos")).toEqual(["1.6", "1.6", "1.3"]);
});

test("the GNOME emblem draws its sheets at 2 and its fold at 1.5", () => {
  expect(emblemWidths("gnome")).toEqual(["2", "2", "1.5"]);
});

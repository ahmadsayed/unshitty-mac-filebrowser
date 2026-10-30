// The two icon sets (#41): the macOS theme redraws every icon under the same
// symbol ids, so the views' markup is the same in both themes.
import { beforeAll, expect, test } from "vitest";
import { spriteMarkup } from "../src/icons";
import { boot, item } from "./app";

const ids = (markup: string): string[] => {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.innerHTML = markup;
  return [...svg.querySelectorAll("symbol")].map((s) => s.id).sort();
};
const sprite = () => document.getElementById("icon-sprite")!;

beforeAll(boot);

test("the macOS set draws every icon the GNOME set has, under the same ids", () => {
  const gnome = ids(spriteMarkup("gnome"));
  expect(gnome.length).toBeGreaterThan(60);
  expect(ids(spriteMarkup("macos"))).toEqual(gnome);
});

test("folders, files and the sidebar's line icons are redrawn; the rest keep GNOME's shape", () => {
  const symbols = (theme: "gnome" | "macos") => {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.innerHTML = spriteMarkup(theme);
    return (id: string) => svg.querySelector(`#${id}`)!.innerHTML;
  };
  const gnome = symbols("gnome");
  const macos = symbols("macos");
  for (const id of ["fc-folder-plain", "fc-folder-documents", "fc-folder-back", "fc-file-text", "fc-file-image", "fc-file-archive", "fc-file-disk", "fc-file-executable", "sym-user-home", "sym-user-trash", "sym-network-workgroup", "sym-view-more", "sym-emblem-symbolic-link"]) {
    expect(macos(id), id).not.toBe(gnome(id));
  }
  // Not redrawn: the macOS theme keeps these as they are.
  for (const id of ["sym-network-server", "sym-media-eject", "sym-emblem-versions", "sym-window-close"]) {
    expect(macos(id), id).toBe(gnome(id));
  }
});

test("the app installs one sprite, with its platform's icons", () => {
  // The test setup's Mac user agent: the macOS theme.
  expect(document.querySelectorAll("#icon-sprite")).toHaveLength(1);
  expect(sprite().dataset.theme).toBe("macos");
  const macFolder = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  macFolder.innerHTML = spriteMarkup("macos");
  expect(sprite().querySelector("#fc-folder-plain")!.innerHTML).toBe(macFolder.querySelector("#fc-folder-plain")!.innerHTML);
  expect(item("Documents").querySelector("use")!.getAttribute("href")).toBe("#fc-folder-documents");
});

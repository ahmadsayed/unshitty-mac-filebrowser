// The two icon sets (#41): the macOS theme redraws every icon under the same
// symbol ids, and a theme switch swaps the sprite's drawings in place.
import { beforeAll, expect, test } from "vitest";
import { spriteMarkup } from "../src/icons";
import { $, boot, chooseMenu, click, item, settle } from "./app";

const ids = (markup: string): string[] => {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.innerHTML = markup;
  return [...svg.querySelectorAll("symbol")].map((s) => s.id).sort();
};
const sprite = () => document.getElementById("icon-sprite")!;
const symbol = (id: string) => sprite().querySelector(`#${id}`)!.innerHTML;

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

test("the app starts with its theme's icons and a switch swaps them in place", async () => {
  // The test setup's Mac user agent: Automatic is macOS.
  expect(sprite().dataset.theme).toBe("macos");
  const macFolder = symbol("fc-folder-plain");
  const use = item("Documents").querySelector("use")!;
  const href = use.getAttribute("href");

  await click($("#btn-main-menu"));
  await chooseMenu("GNOME");
  await settle();
  expect(sprite().dataset.theme).toBe("gnome");
  expect(symbol("fc-folder-plain")).not.toBe(macFolder);
  // Only one sprite, and the items still point at the same symbols.
  expect(document.querySelectorAll("#icon-sprite")).toHaveLength(1);
  expect(item("Documents").querySelector("use")!.getAttribute("href")).toBe(href);

  await click($("#btn-main-menu"));
  await chooseMenu("macOS");
  await settle();
  expect(symbol("fc-folder-plain")).toBe(macFolder);
});

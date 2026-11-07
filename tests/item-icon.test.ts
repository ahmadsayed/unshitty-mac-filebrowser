// Which emblems show on which picture, and in what order (itemIcon in
// src/icons.ts): link, versions, shared; a glyph carries none, a thumbnail
// only the link; a folder the app draws is stacked instead of badged.
import { expect, test } from "vitest";
import { itemIcon, type Emblems, type IconBase } from "../src/icons";

const entry = (name: string, is_dir: boolean) =>
  ({ name, path: `/x/${name}`, is_dir, size: 0, modified: 0, is_hidden: false, is_symlink: false });
const FILE = entry("a.txt", false);
const FOLDER = entry("Kept", true);
const APP = entry("Calc.app", true);

const sym = (id: string) => `<svg class="sym " viewBox="0 0 16 16" aria-hidden="true"><use href="#sym-${id}"/></svg>`;
const LINK = `<span class="emblem">${sym("emblem-symbolic-link")}</span>`;
const BADGE = `<span class="emblem emblem-versions" title="Keeps versions">${sym("emblem-versions")}</span>`;
const SHARED = `<span class="emblem">${sym("network-workgroup")}</span>`;
const drawn = (id: string) => `<svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#${id}"/></svg>`;
const STACKED = `<span class="versioned-icon" title="Keeps versions"><svg class="fc" viewBox="0 0 64 64" aria-hidden="true"><use href="#fc-folder-back" transform="translate(5 -5)"/><use href="#fc-folder-plain"/></svg></span>`;
const OS = `<img class="native-icon" src="blob:os" alt="" draggable="false">`;
const THUMB = `<img class="thumb" src="blob:thumb" alt="" draggable="false">`;
const GLYPH = `<span class="server-glyph">${sym("network-server")}</span>`;

const ALL: Emblems = { link: true, versions: true, shared: true };
const NONE: Emblems = { link: false, versions: false, shared: false };

test.each<[string, ReturnType<typeof entry>, IconBase, Emblems, string]>([
  ["a glyph carries no emblem", FOLDER, { glyph: "network-server" }, ALL, GLYPH],
  ["a thumbnail carries only the link", FILE, { thumb: "blob:thumb" }, ALL, THUMB + LINK],
  ["a thumbnail without a link carries nothing", FILE, { thumb: "blob:thumb" }, { link: false, versions: true, shared: true }, THUMB],
  ["the OS icon: link, badge, shared", FOLDER, { native: "blob:os" }, ALL, OS + LINK + BADGE + SHARED],
  ["the OS icon alone", FILE, { native: "blob:os" }, NONE, OS],
  ["a drawn file: link, badge, shared", FILE, { drawn: "plain" }, ALL, drawn("fc-file-text") + LINK + BADGE + SHARED],
  ["a drawn folder is stacked, then link and shared", FOLDER, { drawn: "plain" }, ALL, STACKED + LINK + SHARED],
  ["a drawn folder that keeps versions is stacked", FOLDER, { drawn: "plain" }, { link: false, versions: true, shared: false }, STACKED],
  ["a drawn app is badged, not stacked", APP, { drawn: "plain" }, ALL, drawn("fc-file-executable") + LINK + BADGE + SHARED],
  ["a drawn folder of a kind, shared", FOLDER, { drawn: "music" }, { link: false, versions: false, shared: true }, drawn("fc-folder-music") + SHARED],
])("%s", (_, e, base, emblems, expected) => {
  expect(itemIcon(e, base, emblems)).toBe(expected);
});

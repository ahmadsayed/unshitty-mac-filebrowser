# macOS icons

**Date:** 2026-10-30

**Status:** Implemented; not verified on a Mac.

**Issue:** #41

**Amends:** [Themes](2026-10-29-themes-design.md), whose non-goal
"Mac-style file and folder icons" this resolves.

## Goal

With the macOS theme, the app looks at home on a Mac down to its icons:
light-blue folders with a tab, white file pages with a folded corner and a
coloured mark per kind, and thin line icons in the sidebar and header bar.
The GNOME theme keeps its icons exactly as they are, pixel for pixel.

## Rules

- **Original drawings only.** No Apple icons, SF Symbols, or anything traced
  from them; no GNOME or Adwaita drawings either. The shapes are generic
  (a folder, a page, a globe, a monitor) drawn from scratch as SVG paths.
- **Same ids.** The macOS set draws every symbol the GNOME set has, under the
  same id (`fc-folder-plain`, `sym-user-home`, …). Nothing that renders an
  icon knows which theme is active; markup and `<use href>` never change.
- **Colours** are part of the drawing, like the GNOME full-colour icons.
  Symbolic icons stay `currentColor`, so they follow the theme's tokens and
  dark mode by themselves. Full-colour icons look the same in light and dark,
  as on a Mac.

## How it works

- `src/icons.ts` builds the sprite with `spriteMarkup(theme)`. For GNOME it is
  the markup the app always had. For macOS it is the GNOME symbolic set with
  the redrawn line icons laid over it (`{...SYMBOLIC, ...MACOS_SYMBOLIC}`),
  plus the macOS folders, versioned-folder back layer and file pages.
- `installIconSprite(theme)` creates the one `#icon-sprite` at start-up with
  the resolved theme. `setIconTheme(theme)`, called from `applyTheme()`,
  replaces the sprite's contents when the theme changes, so every icon on
  screen redraws at once without re-rendering the views. Same theme: no-op.
- `src/icons-macos.ts` holds the drawings: `MACOS_SYMBOLIC`, the gradients
  (`MACOS_DEFS`), `macFolderSymbol`, `macFolderBackLayer`, `macFileSymbol`.
- `src/themes/macos.css` rounds the emblem badge (the symlink and versions
  marks) into a small rounded square, like a Finder alias badge.

## The set

**Folders:** a light-blue folder with a darker back panel and tab; special
folders (Documents, Music, Pictures, Videos, Downloads, Desktop, Public,
Templates, Home) carry their glyph embossed on the front in a darker blue.
The versioned folder shows a second, darker folder behind.

**Files:** a white page with a folded corner and a soft shadow, marked by
kind: grey lines (text), indigo brackets (code), pink note (audio), purple
play tile (video), red band (PDF), blue header (document), green grid
(spreadsheet), orange slide (presentation). Images are a framed landscape,
archives a page with a zipper, disk images a silver drive, executables a blue
app tile.

**Line icons** (1.3 px strokes, round caps): back, forward, the disclosure
chevrons, search, the main menu, ⋯ in a circle, grid and list, New Folder,
the sidebar places (Home, Recent, Starred, Network as a globe, Trash, the
special folders, Desktop as a monitor, Downloads as an arrow in a circle),
the symlink badge as a curved alias arrow, the hard drive.

**Kept as they are:** the window controls, eject, server, the versions
emblem, and the rest of the symbolic set. They read well on a Mac already;
redrawing them is a later choice, not a gap.

## Verification

- `tests/icons.test.ts`: both sets have the same ids; the listed icons are
  redrawn and the kept ones are identical; the app starts with its theme's
  set (Mac user agent → macOS); switching themes swaps the drawings in the
  one sprite and leaves the `<use>` hrefs alone.
- GNOME theme: every mock screenshot scene (grid, list, menu, dialog, light
  and dark) is byte-identical to `main`.
- **Not verified on a Mac:** how the icons look next to real macOS chrome, at
  Retina scale, and with SF Pro labels.

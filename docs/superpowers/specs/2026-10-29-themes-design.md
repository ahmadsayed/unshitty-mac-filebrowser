# Themes: GNOME and macOS

**Date:** 2026-10-29

**Status:** Approved design, not implemented.

**Amends:** [File Browser design](2026-10-23-file-browser-design.md) — the
look is no longer GNOME Files only.

## Goal

The app gets a choice of look. Today it imitates GNOME Files 50 everywhere.
A second theme imitates **macOS 26 (Tahoe)** and is the default on a Mac, so
the app looks at home there. Linux keeps the GNOME look by default.

A theme changes only the **look**: colours, fonts, sizes, corner radii,
translucency. Behaviour and wording stay GNOME's in both themes: the same
menus and labels, shortcuts, dialog button order and default responses, the
same Trash, clipboard and drag-and-drop rules. Nothing a theme does can
affect files.

Success: on a Mac the window reads as a Tahoe app (floating glass sidebar,
capsule toolbar controls, system font and blue accent); switching theme in
the main menu restyles the open window and every other window instantly,
keeping the folder, selection, scroll position and tabs; the GNOME theme is
pixel-identical to the app before this change.

## Non-goals

- **Native window vibrancy** (the desktop tinting the sidebar through an
  `NSVisualEffectView`). It needs a transparent window and `macOSPrivateApi`
  and can only be verified on a Mac. Follow-up, own PR and issue.
- Mac conventions where they differ from GNOME: button order, menu wording,
  Finder's sidebar headings ("Favorites", "Locations"), a bottom path bar.
- Mac-style file and folder icons. The theme keeps the app's own icons;
  new drawings are a separate task.
- More than two themes, user-made themes, accent colour choice.
- Any Apple asset. The theme uses the system font through `-apple-system`
  (SF Pro on a Mac) and draws nothing from Apple; it is an original imitation.

## Choosing a theme

`prefs.theme` is `"auto" | "gnome" | "macos"`, default `"auto"`, kept in
localStorage like the other UI preferences (per machine, not synced).
`"auto"` resolves to `macos` when `isMac`, else `gnome`. Both themes are
available on both platforms.

The main menu gets a **Theme** block between "Show Thumbnails" and
"Keyboard Shortcuts": a `.menu-heading` "Theme" and three radio items,
**Automatic**, **GNOME**, **macOS**, built with the radio items `openMenu`
already supports. Picking one saves the pref and applies it.

The resolved theme is written as `data-theme="gnome|macos"` on `<html>`
before the first paint (the same place `platform-mac` is set at startup), so
a window never shows the wrong theme first.

## Switching instantly

`applyTheme()` is the only place that changes the theme:

1. set `data-theme` on `<html>`;
2. recompute the virtualized layout (`relayout()`) and re-render the visible
   window, keeping the first visible item at the top;
3. nothing else: no reload, no crossfade animation.

CSS restyles everything else in the same frame. The folder, selection,
cursor, scroll position, tabs and open dialogs stay as they are. An open
popover or menu in another window closes, since its anchor moves.

Other windows follow: every window listens to the `storage` event for
`prefs` and calls `applyTheme()` when the resolved theme changed.

The file views are virtualized and position items from JavaScript
(`computeLayout`, `itemPos`), so the layout sizes that differ by theme move
from constants into a per-theme table read by `computeLayout`:

| Size | GNOME | macOS |
|---|---|---|
| List row padding (row = icon + padding) | 12 px (today) | 8 px |
| Minimum list row height | 34 px (today) | 24 px |
| List header height | 35 px (today's `LIST_HEADER_H`) | 28 px |
| Grid padding / gap | 12 / 6 px (today) | 12 / 6 px |

Hit-testing (rubber band, drop targets) already uses the layout maths, so it
follows without change. The font size is already read from CSS.

The traffic lights' position is fixed in `tauri.conf.json` and cannot move at
runtime; the macOS sidebar is laid out so they fall inside its top-left corner.

## Stylesheet structure

- `src/styles.css` keeps all structure (layout, views, widgets) and reads
  every look-related value from tokens.
- `src/themes/gnome.css`: today's libadwaita colour values, light and dark,
  moved out of `styles.css` unchanged, plus the new size and shape tokens at
  today's values. Selector `:root, :root[data-theme="gnome"]`, so anything
  rendered before the attribute is set, and the tests, get the GNOME look.
- `src/themes/macos.css`: the macOS values, light and dark, under
  `:root[data-theme="macos"]`, plus a short block of rules scoped to
  `[data-theme="macos"]` for what tokens can't express (the floating sidebar
  panel, glass surfaces, capsule groups, list stripes).

New tokens, only where the two themes differ:

- radii: `--radius-window`, `--radius-button`, `--radius-popover`,
  `--radius-dialog`, `--radius-item`;
- metrics: `--headerbar-height`, `--sidebar-row-height`, `--list-row-height`,
  `--button-height`;
- type: `--font` (exists), `--font-size`;
- glass: `--glass-bg`, `--glass-blur` (GNOME: the opaque surface colour and
  `0`, so no blur).

Values that don't differ (1 px borders, small gaps) stay literal.

Dark mode follows `prefers-color-scheme` in both themes. Every colour is a
token with a dark value.

## The macOS theme

Following Tahoe's Finder:

**Window and sidebar.** The sidebar is a floating panel inset 8 px from the
window edges, 18 px corners, glass surface (translucent tint + backdrop
blur). The traffic lights sit in its top-left corner. Rows 28 px, 13 px
text, accent-coloured icons, a rounded grey highlight for the selected row.
The separators between sections stay.

**Toolbar.** The header bar has no background or bottom border of its own;
the file view scrolls under it and the glass blurs it. Back/Forward form one
glass capsule, the view toggle and view menu another, each about 32 px high.
The path bar and the search entry are capsules; their contents and behaviour
don't change. The tab bar uses the same capsules.

**Files.** System font (`-apple-system`, then Inter, then Adwaita Sans) at
13 px. Accent `#007AFF` light, `#0A84FF` dark. Grid: a selected item gets a
rounded grey box around the icon and a blue pill with white text around the
name. List: 24 px rows, alternating stripes, a selected row filled solid blue
with white text, smaller and lighter column headers.

**Menus, dialogs, toasts, banners.** Menus and popovers: glass, 12 px
corners, no arrow, 24 px items, blue full-width hover highlight. Dialogs:
24 px corners, capsule buttons, GNOME's button order and default response.
Toasts: a glass capsule. Banners: inset with rounded corners, lighter.

**On Linux** the macOS theme keeps the Linux window buttons, restyled as
small round buttons to fit.

## Delivery

1. **Token refactor, GNOME only.** Move the colour values to
   `themes/gnome.css`, add the size/shape tokens and the per-theme layout
   table at today's values. Must be **pixel-identical**: mock screenshots of
   fixed scenes (grid, list with a selection, an open menu, a dialog; light
   and dark) from `main` and from the branch, compared with a pixel diff that
   must be zero.
2. **Theme switching.** `prefs.theme`, `applyTheme()`, the menu block, the
   `storage` listener.
3. **macOS theme.** `themes/macos.css` and its layout sizes.
4. **Docs** (below).

One PR, with these as separate commits.

## Verification

**Frontend tests** (`tests/theme.test.ts`):

- with a Mac user agent and no saved pref, `data-theme` is `macos`; with a
  Linux one, `gnome`;
- choosing GNOME or macOS from the main menu sets the attribute, saves the
  pref and survives a fresh boot; Automatic follows the platform again;
- the menu shows the current choice as the checked radio item;
- after switching, list rows are positioned at the new theme's row height,
  the selection is unchanged and the first visible item stays at the top;
- a `storage` event carrying another theme switches this window.

jsdom applies no CSS, so the look is checked with screenshots.

**Screenshots.** `scripts/mock-screenshot.mjs` gains `--theme gnome|macos`
and the fixed scenes above. The PR shows the macOS theme light and dark next
to GNOME, plus the zero pixel diff of step 1.

**Not verified from Linux** (a `needs-verification` issue, one line each):
the macOS theme in a real Mac window: WebKit's `backdrop-filter` rendering,
SF Pro metrics, the traffic lights inside the floating sidebar, dragging the
window by the transparent header, Automatic picking macOS on a Mac. Native
vibrancy gets its own follow-up issue.

## Docs to update

- **AGENTS.md:** "Look and behave like GNOME Files 50" becomes: the GNOME
  theme follows GNOME Files 50, the macOS theme follows macOS 26 Tahoe;
  wording and behaviour stay GNOME's in both; themes live in `src/themes/`;
  a new look-related value is a token with a value in each theme.
- **README:** themes in the feature list; native vibrancy, Mac-style icons
  and sidebar headings under "Not supported yet"; the independence and
  trademark section gains a line that the macOS theme is an original
  imitation that uses no Apple assets.

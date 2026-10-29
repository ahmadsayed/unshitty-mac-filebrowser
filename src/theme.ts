// ---------------------------------------------------------------------------
// Themes: the look only. GNOME follows GNOME Files 50, macOS follows macOS 26
// (Tahoe); behaviour and wording are GNOME's in both. The colours, fonts and
// shapes are CSS tokens in src/themes/; the sizes the virtualized views
// compute in JavaScript are here.
// ---------------------------------------------------------------------------

/** What the user picked in the main menu. */
export type ThemeChoice = "auto" | "gnome" | "macos";
/** The look actually shown, written as `data-theme` on `<html>`. */
export type Theme = "gnome" | "macos";

/** A saved choice, or "auto" for anything else (an old or garbled pref). */
export function themeChoice(value: unknown): ThemeChoice {
  return value === "gnome" || value === "macos" ? value : "auto";
}

/** The theme a choice shows: "auto" is macOS on a Mac, GNOME elsewhere. */
export function resolveTheme(choice: unknown, isMac: boolean): Theme {
  const picked = themeChoice(choice);
  if (picked !== "auto") return picked;
  return isMac ? "macos" : "gnome";
}

/** Layout sizes of the file views, in CSS pixels. */
export interface LayoutSizes {
  /** List row height = icon size + this padding… */
  listPad: number;
  /** …but never less than this. */
  listMinRow: number;
  /** The sticky list header with its bottom margin (`--list-header-height` + 3). */
  listHeader: number;
  /** Grid padding around the items, and the gap between them. */
  gridPad: number;
  gridGap: number;
  /** Grid icon size on a fresh start and after ⌘0 (an index into GRID_SIZES:
   *  1 is 64 px, 2 is 96 px). A zoom the user chose is kept. */
  gridZoom: number;
}

export const LAYOUT_SIZES: Record<Theme, LayoutSizes> = {
  gnome: { listPad: 12, listMinRow: 34, listHeader: 35, gridPad: 12, gridGap: 6, gridZoom: 2 },
  macos: { listPad: 8, listMinRow: 24, listHeader: 28, gridPad: 12, gridGap: 6, gridZoom: 1 },
};

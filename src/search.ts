// The search: how names are compared. No DOM.

/**
 * Text as search compares it: lower case, without accents. Combining marks
 * are stripped after canonical decomposition, exactly as the backend's
 * `fs_ops::fold` does, so the quick filter here and the recursive search
 * agree; `tests/fold-cases.json` holds the cases both are tested against.
 */
export function fold(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

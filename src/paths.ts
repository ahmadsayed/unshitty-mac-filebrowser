// Pure path rules shared by navigation and the path bar. Kept free of DOM and
// app state so every branch can be tested directly.

/** Every Google Drive location starts with this. */
export const CLOUD_PREFIX = "gdrive://";

export function isCloudPath(path: string): boolean {
  return path.startsWith(CLOUD_PREFIX);
}

/** The account key in `gdrive://<account>/…`. */
export function cloudAccountKey(path: string): string {
  return path.slice(CLOUD_PREFIX.length).split("/")[0] ?? "";
}

/** Resolve what the user typed in the path bar. */
export function resolveTypedPath(
  input: string,
  location: string,
  home: string
): { path: string; error?: undefined } | { path?: undefined; error: string } {
  let typed = input.trim();
  if (!typed) return { path: "" };

  // A pasted Drive link becomes a location for the current account.
  if (isCloudPath(location) && /^https:\/\/drive\.google\.com\//.test(typed)) {
    const id =
      typed.match(/\/d\/([A-Za-z0-9_-]+)/)?.[1] ??
      typed.match(/folders\/([A-Za-z0-9_-]+)/)?.[1];
    if (id) typed = `${CLOUD_PREFIX}${cloudAccountKey(location)}/${id}`;
  }

  if (typed === "~" || typed.startsWith("~/")) {
    typed = home + typed.slice(1);
  } else if (
    isCloudPath(location) &&
    !isCloudPath(typed) &&
    !typed.startsWith("/") &&
    !location.startsWith("pseudo:")
  ) {
    // A bare name inside Drive has no id, so it cannot be resolved safely.
    return { error: "Enter a Google Drive location or a full path." };
  } else if (
    !typed.startsWith("/") &&
    !location.startsWith("pseudo:") &&
    !isCloudPath(location) &&
    !isCloudPath(typed)
  ) {
    typed = `${location.replace(/\/+$/, "")}/${typed}`;
  }

  return { path: typed.length > 1 ? typed.replace(/\/+$/, "") : typed };
}

// Pure path rules shared by navigation and the path bar. Kept free of DOM and
// app state so every branch can be tested directly.

/** Every Google Drive location starts with this. */
export const CLOUD_PREFIX = "gdrive://";

/** Every received-share location starts with this. */
export const SHARE_PREFIX = "share://";

/** Every SSH/SFTP location starts with this. */
export const SFTP_PREFIX = "sftp://";

export function isCloudPath(path: string): boolean {
  return path.startsWith(CLOUD_PREFIX);
}

export function isSharePath(path: string): boolean {
  return path.startsWith(SHARE_PREFIX);
}

export function isSftpPath(path: string): boolean {
  return path.startsWith(SFTP_PREFIX);
}

/** A link from Share (also accepts the `<fbshare1:…>` form people paste). */
export function isShareLink(text: string): boolean {
  let link = text.trim();
  if (link.startsWith("<") && link.endsWith(">")) link = link.slice(1, -1).trim();
  return link.toLowerCase().startsWith("fbshare1:");
}

/** Drive, received shares and SFTP are all remote, but different things. */
export function isRemotePath(path: string): boolean {
  return isCloudPath(path) || isSharePath(path) || isSftpPath(path);
}

/** A share path for a child named `name`, encoded like a Drive name. */
export function joinSharePath(dir: string, name: string): string {
  return `${dir.replace(/\/+$/, "")}/${encodeCloudName(name)}`;
}

/** An SFTP path for a child named `name`, encoded like a Drive name. */
export function joinSftpPath(dir: string, name: string): string {
  return `${dir.replace(/\/+$/, "")}/${encodeCloudName(name)}`;
}

/** The authority (user@host:port) of an sftp:// path, for server lookups. */
export function sftpAuthority(path: string): string {
  return path.slice(SFTP_PREFIX.length).split("/")[0] ?? "";
}

/** The account key in `gdrive://<account>/…`. */
export function cloudAccountKey(path: string): string {
  return path.slice(CLOUD_PREFIX.length).split("/")[0] ?? "";
}

/**
 * The Drive path of a child named `name` that may not exist yet: a pending
 * `~name` segment. The name is encoded exactly as the backend's `NAME_SET`
 * does: every byte except ASCII letters, digits, `.`, `-` and `_` as `%XX`.
 */
export function joinCloudPath(dir: string, name: string): string {
  return `${dir.replace(/\/+$/, "")}/~${encodeCloudName(name)}`;
}

/**
 * A Drive name as a local file name, by the backend's `safe_name` rules:
 * `/`, `\`, NUL and control characters become `_`, leading and trailing
 * dots and spaces go, an empty result is "item", and at most 180 characters
 * are kept. Drive allows names like "a/b" and ".." that would escape the
 * destination folder.
 */
export function safeLocalName(name: string): string {
  const cleaned = [...name].map((c) => (c === "/" || c === "\\" || c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127 ? "_" : c)).join("");
  const trimmed = cleaned.replace(/^\.+|\.+$/g, "").trim();
  return trimmed ? [...trimmed].slice(0, 180).join("") : "item";
}

/** A Drive display name as it appears in a `gdrive://` path. */
export function encodeCloudName(name: string): string {
  return [...new TextEncoder().encode(name)]
    .map((byte) => {
      const char = String.fromCharCode(byte);
      return /[A-Za-z0-9._-]/.test(char) ? char : `%${byte.toString(16).toUpperCase().padStart(2, "0")}`;
    })
    .join("");
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
  } else if (isSftpPath(location) && !isSftpPath(typed)) {
    typed = typed.startsWith("/")
      ? `${SFTP_PREFIX}${sftpAuthority(location)}${typed}`
      : `${location.replace(/\/+$/, "")}/${typed}`;
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
    !isCloudPath(typed) &&
    !isSftpPath(typed)
  ) {
    typed = `${location.replace(/\/+$/, "")}/${typed}`;
  }

  return { path: typed.length > 1 ? typed.replace(/\/+$/, "") : typed };
}

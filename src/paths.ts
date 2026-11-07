// Pure path rules shared by navigation and the path bar: the layout of each
// kind of path (Google Drive, received shares, SSH servers, local), their
// names, parents, children and crumbs, containment, and which paths a window
// may open. Kept free of DOM and app state so every branch can be tested
// directly (tests/paths.test.ts).
//
// The strings are the backend's: stars, pins and history store them, so
// nothing here rewrites a path Rust produced.

/** Every pseudo location (Recent, Starred, Network) starts with this. */
export const PSEUDO_PREFIX = "pseudo:";

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

/**
 * Received shares live at `share://<16-character id>/…`; a share link is
 * `share://` followed by its whole ticket: 64 or more base32 characters and
 * nothing after them.
 */
const LINK_TICKET = /^[a-z2-7]{64,}\/?$/;

/**
 * The ticket inside a share link, lowercased, with or without the `<…>` and
 * trailing slash people paste; null when `text` isn't a link. Two links to
 * the same share have the same ticket however they were pasted.
 */
export function shareTicket(text: string): string | null {
  let link = text.trim();
  if (link.startsWith("<") && link.endsWith(">")) link = link.slice(1, -1).trim();
  link = link.toLowerCase();
  if (!link.startsWith(SHARE_PREFIX)) return null;
  const ticket = link.slice(SHARE_PREFIX.length);
  return LINK_TICKET.test(ticket) ? ticket.replace(/\/$/, "") : null;
}

/** A link from Share (also accepts the `<share://…>` form people paste). */
export function isShareLink(text: string): boolean {
  return shareTicket(text) !== null;
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

/** A server's root, `sftp://<authority>/`: always with its slash, as the
 *  server's URL is written, so Up, the root crumb and Network agree. */
export function sftpRoot(authority: string): string {
  return `${SFTP_PREFIX}${authority}/`;
}

/** A received share's root, `share://<id>`, without a slash. */
function shareRoot(id: string): string {
  return `${SHARE_PREFIX}${id}`;
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
    !location.startsWith(PSEUDO_PREFIX)
  ) {
    // A bare name inside Drive has no id, so it cannot be resolved safely.
    return { error: "Enter a Google Drive location or a full path." };
  } else if (
    !typed.startsWith("/") &&
    !location.startsWith(PSEUDO_PREFIX) &&
    !isCloudPath(location) &&
    !isCloudPath(typed) &&
    !isSftpPath(typed)
  ) {
    typed = `${location.replace(/\/+$/, "")}/${typed}`;
  }

  return { path: withoutTrailingSlash(typed) };
}

/** Typed text without its trailing slashes, except "/" and a server's root,
 *  which keeps exactly one. */
function withoutTrailingSlash(typed: string): string {
  const path = typed.length > 1 ? typed.replace(/\/+$/, "") : typed;
  return isSftpPath(path) && afterPrefix(path, SFTP_PREFIX).length === 1 ? sftpRoot(sftpAuthority(path)) : path;
}

// ---------------------------------------------------------------------------
// Each scheme's layout: names, parents, children and crumbs
// ---------------------------------------------------------------------------
//
//   gdrive://<account>/<id>~<encoded name>/…   (a pending child is `~<name>`)
//   share://<share id>/<encoded name>/…
//   sftp://<authority>/<encoded name>/…        (the server's root is `sftp://<authority>/`)
//   /local/path

/** Which kind of path this is. Anything that isn't Drive, a share or SSH is
 *  "local", pseudo locations included: ask `isPseudo` first where it matters. */
export type Place = "drive" | "share" | "sftp" | "local";

export function placeOf(path: string): Place {
  if (isCloudPath(path)) return "drive";
  if (isSharePath(path)) return "share";
  if (isSftpPath(path)) return "sftp";
  return "local";
}

/** A path broken into its scheme's root and the folders below it. */
export interface PathSegments {
  /** `key` is the account, share id or authority ("" when local); `name` is
   *  the root's decoded key. */
  root: { kind: Place; key: string; path: string; name: string };
  segments: { name: string; path: string }[];
}

/** `segment` with its `%XX` escapes decoded, or as it is when they are malformed. */
export function decodeSegment(segment: string): string {
  try {
    return decodeURIComponent(segment);
  } catch {
    return segment;
  }
}

/** The display name inside a Drive `<file-id>~<encoded name>` segment. */
export function cloudSegmentName(segment: string): string {
  const tilde = segment.indexOf("~");
  return decodeSegment(tilde >= 0 ? segment.slice(tilde + 1) : segment);
}

/** The share id in `share://<id>/…`. */
export function shareId(path: string): string {
  return path.slice(SHARE_PREFIX.length).split("/")[0] ?? "";
}

/** The first folder of an account, whose parent is the account itself. */
export function isCloudRoot(path: string): boolean {
  return isCloudPath(path) && path.slice(CLOUD_PREFIX.length).split("/").filter(Boolean).length <= 1;
}

/** The non-empty segments after `prefix`. */
function afterPrefix(path: string, prefix: string): string[] {
  return path.slice(prefix.length).split("/").filter(Boolean);
}

/** A local child: the exact join every local copy, move, rename and new item uses. */
function localChild(dir: string, name: string): string {
  return dir.replace(/\/+$/, "") + "/" + name;
}

/** The local folders from `base` (left out) down to `path`, each with its path. */
export function localSegments(base: string, path: string): { name: string; path: string }[] {
  const names = path.slice(base.length).split("/").filter(Boolean);
  return names.map((name, i) => ({ name, path: localChild(base, names.slice(0, i + 1).join("/")) }));
}

/** How a remote scheme names its root and its segments. */
interface RemoteShape {
  root: (key: string) => string;
  rootName: (key: string) => string;
  segment: (segment: string) => string;
}

/** A remote path's root (account, share or server) and the segments below it. */
function remoteSegments(path: string, kind: Exclude<Place, "local">, prefix: string, shape: RemoteShape): PathSegments {
  const parts = afterPrefix(path, prefix);
  const key = parts.shift() ?? "";
  let at = `${prefix}${key}`;
  const segments = parts.map((segment) => ({ name: shape.segment(segment), path: (at += `/${segment}`) }));
  return { root: { kind, key, path: shape.root(key), name: shape.rootName(key) }, segments };
}

/** The decoded last segment after `prefix`: the share id or the server's
 *  authority at the root. Malformed escapes are shown as they are. */
function remoteName(path: string, prefix: string): string {
  return decodeSegment(afterPrefix(path, prefix).pop() ?? "");
}

/** Up from a share or server path: its root written as `root` writes it,
 *  null above it. */
function remoteParent(path: string, prefix: string, root: (key: string) => string): string | null {
  const parts = afterPrefix(path, prefix);
  parts.pop();
  if (parts.length === 1) return root(parts[0]);
  return parts.length ? `${prefix}${parts.join("/")}` : null;
}

/** What each scheme decides about its paths. */
interface Scheme {
  name(path: string): string;
  parent(path: string): string | null;
  child(dir: string, name: string): string;
  segments(path: string): PathSegments;
}

const SCHEMES: Record<Place, Scheme> = {
  drive: {
    name: (path) => cloudSegmentName(path.replace(/\/+$/, "").split("/").pop() ?? ""),
    // An account's first folder is its own parent.
    parent: (path) => {
      const trimmed = path.replace(/\/+$/, "");
      const slash = trimmed.lastIndexOf("/");
      return slash > CLOUD_PREFIX.length ? trimmed.slice(0, slash) : trimmed;
    },
    child: joinCloudPath,
    segments: (path) =>
      remoteSegments(path, "drive", CLOUD_PREFIX, {
        root: (key) => `${CLOUD_PREFIX}${key}`,
        rootName: (key) => key,
        segment: cloudSegmentName,
      }),
  },
  share: {
    name: (path) => remoteName(path, SHARE_PREFIX),
    parent: (path) => remoteParent(path, SHARE_PREFIX, shareRoot),
    // today's behaviour, fixed in a follow-up: a share child is joined like a
    // local one, unencoded (joinSharePath would encode it).
    child: localChild,
    segments: (path) =>
      remoteSegments(path, "share", SHARE_PREFIX, {
        root: shareRoot,
        rootName: (key) => key,
        segment: decodeSegment,
      }),
  },
  sftp: {
    name: (path) => remoteName(path, SFTP_PREFIX),
    parent: (path) => remoteParent(path, SFTP_PREFIX, sftpRoot),
    child: joinSftpPath,
    segments: (path) =>
      remoteSegments(path, "sftp", SFTP_PREFIX, {
        root: sftpRoot,
        rootName: decodeSegment,
        segment: decodeSegment,
      }),
  },
  local: {
    name: (path) => path.replace(/\/+$/, "").split("/").pop() ?? "",
    // The root is its own parent.
    parent: (path) => path.replace(/\/+$/, "").replace(/\/[^/]*$/, "") || "/",
    child: localChild,
    segments: (path) => ({ root: { kind: "local", key: "", path: "/", name: "" }, segments: localSegments("/", path) }),
  },
};

/** The decoded last segment of `path`; "" when there is none (the local root). */
export function pathName(path: string): string {
  return SCHEMES[placeOf(path)].name(path);
}

/** The folder holding `path`; null above a share's or a server's root. A
 *  Drive account's first folder and the local root are their own parents. */
export function parentPath(path: string): string | null {
  return SCHEMES[placeOf(path)].parent(path);
}

/** The path of a child named `name` inside `dir`, encoded as the backend
 *  decodes it: a pending `~name` in Drive, `%XX` names on SSH servers. */
export function childPath(dir: string, name: string): string {
  return SCHEMES[placeOf(dir)].child(dir, name);
}

/** `path` broken into its root and the folders below it, for the crumbs. */
export function pathSegments(path: string): PathSegments {
  return SCHEMES[placeOf(path)].segments(path);
}

/** Whether `path` is `dir` itself or inside it; a trailing slash on `dir` doesn't
 *  matter, so "/" holds every absolute path. For roots typed or stored with a
 *  slash (a versioned folder, a folder that just received items). */
export function isUnder(path: string, dir: string): boolean {
  return path === dir || path.startsWith(dir.replace(/\/+$/, "") + "/");
}

/** Whether `path` is `dir` itself or inside it, with `dir` taken as written:
 *  "/" holds only "/", and a `dir` ending in a slash holds only itself. For
 *  Home (which is "/" when HOME=/), the Trash, volumes and items, where "/"
 *  must keep its own crumb and the Location column slices after `dir + "/"`. */
export function isWithin(path: string, dir: string): boolean {
  return path === dir || path.startsWith(dir + "/");
}

/** Whether a window may open `path` from its `?path=` (New Window). */
export function isKnownPath(path: string): boolean {
  // today's behaviour, fixed in a follow-up: sftp:// isn't listed, so New
  // Window from an SSH folder opens Home.
  return path.startsWith("/") || path.startsWith(PSEUDO_PREFIX) || isCloudPath(path) || isSharePath(path);
}

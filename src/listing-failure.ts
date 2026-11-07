// What a folder that couldn't be listed turns into, one table row per kind
// of failure. The same error means different things depending on where the
// listing was asked from:
//
// - "entry": the path entry checks a typed path before going there, so it
//   names the typed path, and an SSH server that isn't connected opens the
//   Connect to Server dialog.
// - "navigate": going somewhere lists before anything changes, so a folder
//   that can't be read keeps the current view and shows a toast.
// - "first": the window's first view has nothing to keep, so it shows the
//   error page.
// - "reload": the view is already there; a local folder that went away
//   goes to its parent, anything else shows the error page.
//
// A received share shows its status page instead (offline, or no access).
// A macOS privacy block ("Operation not permitted") gets Grant Access,
// which opens the Full Disk Access setting. Kept free of DOM and app state
// so every row can be tested directly (tests/listing-failure-table.test.ts).

import { isLocalLocation } from "./locations";
import { isSftpPath, isSharePath } from "./paths";

export type ListingPhase = "first" | "navigate" | "reload" | "entry";

/** A received share's status, as Network last saw it. */
export type ShareState = "online" | "offline" | "no-access" | "missing";

export interface ListingContext {
  phase: ListingPhase;
  isMac: boolean;
  /** For a share:// location, its share's status. */
  share?: ShareState;
  /** A location's name, as the path bar shows it (main.ts's basename). Only
   *  called for the row that needs it: some names can't be decoded. */
  nameOf: (loc: string) => string;
  /** A location's parent, where a folder that went away goes (main.ts's parentOf). */
  parentOf: (loc: string) => string;
}

/** What the view does: a share's status page, the Connect to Server dialog,
 *  go to the parent, a toast (maybe with Grant Access) or the error page. */
export type Reaction =
  | { kind: "sharePage"; status: "offline" | "no-access" }
  | { kind: "connect"; url: string }
  | { kind: "goTo"; parent: string }
  | { kind: "toast"; text: string; grantAccess: boolean }
  | { kind: "errorPage" };

/** A reaction, with the cleaned message for the error page to show or fall back on. */
export type ListingReaction = Reaction & { message: string };

/** "Failed to read directory: Permission denied (os error 13)" → "Permission denied" */
export function cleanError(err: unknown): string {
  return String(err).replace(/^[^:]*:\s*/, "").replace(/\s*\(os error \d+\)$/, "");
}

interface Failure {
  loc: string;
  /** The error as the backend sent it. */
  raw: string;
  /** The error for people: cleanError's. */
  message: string;
  ctx: ListingContext;
}

interface Row {
  when: (f: Failure) => boolean;
  then: (f: Failure) => Reaction;
}

const NOT_PERMITTED = /not permitted/i;
/** ENOENT: the path was removed, not merely unreadable. */
const MISSING = /no such file or directory/i;
/** sftp.rs's answer for a server this window hasn't connected to. */
const NOT_CONNECTED = /Connect to this server first/;

const entry = (f: Failure) => f.ctx.phase === "entry";

/** "Unable to access “Name”: …", with Grant Access for a macOS privacy block. */
function accessToast(name: string, f: Failure): Reaction {
  return { kind: "toast", text: `Unable to access “${name}”: ${f.message}`, grantAccess: f.ctx.isMac && NOT_PERMITTED.test(f.message) };
}

/** First match wins. */
const ROWS: Row[] = [
  // A share: reload says offline; going there trusts the status Network just refreshed.
  { when: (f) => isSharePath(f.loc) && f.ctx.phase === "reload", then: () => ({ kind: "sharePage", status: "offline" }) },
  {
    when: (f) => isSharePath(f.loc) && !entry(f),
    then: (f) => ({ kind: "sharePage", status: f.ctx.share === "no-access" ? "no-access" : "offline" }),
  },
  // The path entry names the path as typed.
  { when: (f) => entry(f) && isSftpPath(f.loc) && NOT_CONNECTED.test(f.message), then: (f) => ({ kind: "connect", url: f.loc }) },
  { when: (f) => entry(f) && NOT_PERMITTED.test(f.message), then: (f) => accessToast(f.loc, f) },
  { when: entry, then: (f) => ({ kind: "toast", text: `Unable to find “${f.loc}”: ${f.message}`, grantAccess: false }) },
  // A cancelled copy deletes the folder it was writing, and the view can
  // still be showing it: go to its parent rather than strand it on an error page.
  { when: (f) => f.ctx.phase === "reload" && isLocalLocation(f.loc) && MISSING.test(f.raw), then: (f) => ({ kind: "goTo", parent: f.ctx.parentOf(f.loc) }) },
  // Going somewhere keeps the view.
  { when: (f) => f.ctx.phase === "navigate", then: (f) => accessToast(f.ctx.nameOf(f.loc), f) },
  // The first view and a reload: the error page.
  { when: () => true, then: () => ({ kind: "errorPage" }) },
];

/** What the view does when listing `loc` failed with `err`. */
export function listingReaction(loc: string, err: unknown, ctx: ListingContext): ListingReaction {
  const failure: Failure = { loc, raw: String(err), message: cleanError(err), ctx };
  const row = ROWS.find((r) => r.when(failure))!;
  return { ...row.then(failure), message: failure.message };
}

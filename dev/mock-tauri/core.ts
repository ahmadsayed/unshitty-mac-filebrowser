import { encodeCloudName } from "../../src/paths";
// Browser-only stand-in for @tauri-apps/api/core, used by `npm run dev:mock`
// so the UI can be previewed in a normal browser without the Rust backend.
// It serves a fake, in-memory home folder.

interface Entry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: number;
  is_hidden: boolean;
  is_symlink: boolean;
  cloud?: {
    account: string;
    id: string;
    mime_type: string;
    is_google_doc: boolean;
    is_shortcut: boolean;
    web_url?: string;
    can_download: boolean;
  };
}

type Node = { dir: true; children: Record<string, Node>; modified: number; link?: boolean } | { dir: false; size: number; modified: number; link?: boolean };

const now = Math.floor(Date.now() / 1000);
const day = 86_400;
const f = (size: number, ago: number, link = false): Node => ({ dir: false, size, modified: now - ago, link });
const d = (children: Record<string, Node> = {}, ago = 3 * day, link = false): Node => ({ dir: true, children, modified: now - ago, link });

const files = (prefix: string, n: number, ext: string): Record<string, Node> =>
  Object.fromEntries(Array.from({ length: n }, (_, i) => [`${prefix} ${i + 1}.${ext}`, f(1000 * (i + 3) ** 3, i * 9 * day)]));

const tree: Record<string, Node> = {
  Users: d({
    me: d({
      Desktop: d({ "todo.txt": f(312, 600) }),
      Documents: d({
        "Quarterly report.pdf": f(2_480_331, 2 * day),
        "Budget 2026.xlsx": f(48_211, 40 * day),
        "Roadmap.key": f(9_810_220, 400 * day),
        "Notes.md": f(4_120, 3600),
        Projects: d({ "unshitty-mac-filebrowser": d({ "README.md": f(2_200, 60) }), website: d() }),
        Versioned: d({
          "notes.txt": f(2_100, 2 * 3600),
          "image.png": f(120_000, day),
          "large.txt": f(2_000_000, day),
          ".versions": d({ "index.sqlite": f(12_000, 3600) }),
        }),
      }, 1 * day),
      Downloads: d({
        "Firefox 150.dmg": f(138_221_004, 5 * day),
        "invoice-0423.pdf": f(88_120, 2 * 3600),
        "photos-export.zip": f(712_993_112, 1 * day),
        "setup.sh": f(1_820, 20 * day),
      }, 2 * 3600),
      Movies: d({ "Holiday.mov": f(1_220_448_301, 90 * day) }),
      Music: d({ ...files("Track", 4, "flac") }),
      Pictures: d({ ...files("IMG_20", 6, "heic"), Screenshots: d(files("Screenshot", 3, "png")), Camera: d(files("DSC", 400, "jpg")) }),
      Public: d(),
      "Empty folder": d({}, 30 * day),
      "a-very-long-folder-name-that-needs-to-wrap-onto-two-lines": d(),
      "main.rs": f(18_400, 10 * 60),
      "song.mp3": f(5_400_000, 6 * day),
      "link-to-docs": d({}, 50 * day, true),
      ".zshrc": f(2_048, 100 * day),
      ".config": d({ "settings.json": f(512, day) }, 10 * day),
      ".Trash": d({ "old.txt": f(42, 9 * day), "unknown origin.zip": f(31_000, 30 * day) }),
    }),
  }),
  // 10k entries for checking large-folder performance: type /big in the path bar.
  big: d({ ...files("file", 9_000, "txt"), ...Object.fromEntries(Array.from({ length: 1_000 }, (_, i) => [`folder ${i}`, d()])) }),
  Volumes: d({ "Macintosh HD": d({}, day, true), "Backup Drive": d() }),
  Applications: d({ "Calculator.app": d({ Contents: d({ "Info.plist": f(1_204, 90 * day) }) }) }),
  System: d(),
};

function lookup(path: string): Node | undefined {
  let node: Node = d(tree);
  for (const part of path.split("/").filter(Boolean)) {
    if (!node.dir) return undefined;
    node = node.children[part];
    if (!node) return undefined;
  }
  return node;
}

// ---------------------------------------------------------------------------
// A small fake Google Drive, used by the sign-in and browsing preview.
// ---------------------------------------------------------------------------

const DRIVE_ROOT = "gdrive://mock/mock-root~My%20Drive";
const DOC_MIME = "application/vnd.google-apps.document";
const FOLDER_MIME = "application/vnd.google-apps.folder";

/** One Drive item. Ids are stable; the tree is linked by `parent`. */
interface DriveItem {
  id: string;
  name: string;
  parent: string | null;
  isDir: boolean;
  size: number;
  doc: boolean;
  trashed: boolean;
}

const driveItems = new Map<string, DriveItem>();
let driveSeq = 0;
/** Each Drive write, in order, for tests that check what happened first. */
export const driveLog: string[] = [];

function addDriveItem(item: Omit<DriveItem, "trashed" | "doc" | "size"> & Partial<DriveItem>): DriveItem {
  const full: DriveItem = { size: 0, doc: false, trashed: false, ...item };
  driveItems.set(full.id, full);
  return full;
}

addDriveItem({ id: "mock-root", name: "My Drive", parent: null, isDir: true });
addDriveItem({ id: "folder-1", name: "Documents", parent: "mock-root", isDir: true });
addDriveItem({ id: "file-1", name: "hello.txt", parent: "mock-root", isDir: false, size: 12 });
addDriveItem({ id: "file-2", name: "Report.docx", parent: "mock-root", isDir: false, size: 4096 });
addDriveItem({ id: "file-3", name: "Plan", parent: "mock-root", isDir: false, doc: true });
addDriveItem({ id: "file-4", name: "nested.txt", parent: "folder-1", isDir: false, size: 3 });

function drivePath(id: string): string {
  const parts: string[] = [];
  for (let item = driveItems.get(id); item; item = item.parent ? driveItems.get(item.parent) : undefined) {
    parts.unshift(`${item.id}~${encodeCloudName(item.name)}`);
  }
  return `gdrive://mock/${parts.join("/")}`;
}

/** The ids along a Drive path and, for a pending `~name` tail, that name. */
function parseDrive(path: string): { ids: string[]; pending?: string } {
  const segments = path.slice("gdrive://mock/".length).split("/").filter(Boolean);
  const last = segments[segments.length - 1] ?? "";
  if (last.startsWith("~")) {
    return { ids: segments.slice(0, -1).map((s) => s.split("~")[0]), pending: decodeURIComponent(last.slice(1)) };
  }
  return { ids: segments.map((s) => s.split("~")[0]) };
}

function driveItemAt(path: string): DriveItem | undefined {
  const { ids, pending } = parseDrive(path);
  if (pending !== undefined) return undefined;
  const item = driveItems.get(ids[ids.length - 1]);
  return item && !item.trashed ? item : undefined;
}

function driveChildren(parentId: string, name?: string): DriveItem[] {
  return [...driveItems.values()].filter(
    (item) => item.parent === parentId && !item.trashed && (name === undefined || item.name === name),
  );
}

/** Split a pending destination into its parent folder and the new name. */
function driveTarget(path: string): { parent: DriveItem; name: string } {
  const { ids, pending } = parseDrive(path);
  const parent = driveItems.get(ids[ids.length - 1]);
  if (pending === undefined || !parent?.isDir || parent.trashed) throw "The destination must be a new name in a Google Drive folder.";
  return { parent, name: pending };
}

function driveEntry(item: DriveItem): Entry {
  return {
    name: item.name,
    path: drivePath(item.id),
    is_dir: item.isDir,
    size: item.size,
    modified: now - day,
    is_hidden: false,
    is_symlink: false,
    cloud: {
      account: "mock",
      id: item.id,
      mime_type: item.doc ? DOC_MIME : item.isDir ? FOLDER_MIME : "text/plain",
      is_google_doc: item.doc,
      is_shortcut: false,
      web_url: item.doc ? "https://docs.google.com/document/d/mock" : undefined,
      can_download: !item.isDir && !item.doc,
    },
  };
}

function listDrive(path: string): Entry[] {
  const folder = driveItemAt(path);
  if (!folder?.isDir) throw "Google Drive item no longer exists.";
  return driveChildren(folder.id).map(driveEntry);
}

function driveNode(path: string): Entry | undefined {
  const item = driveItemAt(path);
  return item && driveEntry(item);
}

function drivePathExists(path: string): boolean {
  const { ids, pending } = parseDrive(path);
  if (pending !== undefined) return driveChildren(ids[ids.length - 1], pending).length > 0;
  return !!driveItemAt(path);
}

// ---------------------------------------------------------------------------
// A fake received share and sharer, for the Network preview.
// ---------------------------------------------------------------------------

const SHARE_FILES: Record<string, { name: string; isDir: boolean; size: number }[]> = {
  "share://r1": [
    { name: "hello.txt", isDir: false, size: 5 },
    { name: "Photos", isDir: true, size: 0 },
  ],
  "share://r1/Photos": [{ name: "trip.jpg", isDir: false, size: 10 }],
};

let mockShareName: string | null = null;
let mockShares: Array<{
  id: string;
  path: string;
  access: Array<{ id: string; kind: "anyone" | "person"; name: string; link: string; bound: boolean }>;
}> = [];
let mockReceived: Array<{ id: string; link: string; folder: string; sharer: string }> = [];
let mockShareSeq = 0;
let mockLinkSeq = 0;
let mockSharersOnline = true;
let mockNoAccess = false;

function listShare(path: string): Entry[] {
  if (!mockSharersOnline) throw "Ahmed is offline.";
  const entries = SHARE_FILES[path];
  if (!entries) throw "The shared folder was moved or deleted.";
  return entries.map((entry) => ({
    name: entry.name,
    path: `${path}/${encodeURIComponent(entry.name)}`,
    is_dir: entry.isDir,
    size: entry.size,
    modified: now - day,
    is_hidden: false,
    is_symlink: false,
  }));
}

function shareNode(path: string): Entry | undefined {
  const at = path.lastIndexOf("/");
  if (at < 0) return undefined;
  try {
    return listShare(path.slice(0, at)).find(
      (entry) => entry.path === path,
    );
  } catch {
    return undefined;
  }
}

function shareProperties(path: string) {
  const entry = shareNode(path);
  if (!entry) throw "Could not read properties: The shared file no longer exists.";
  return {
    name: entry.name,
    path,
    parent: path.slice(0, path.lastIndexOf("/")),
    is_dir: entry.is_dir,
    is_symlink: false,
    symlink_target: null,
    size: entry.size,
    item_count: null,
    modified: entry.modified,
    accessed: entry.modified,
    created: 0,
    mode: "",
    octal: "",
    owner: "Ahmed",
    group: "",
    others: "",
  };
}

function accessView(entry: { id: string; kind: "anyone" | "person"; name: string; bound: boolean }) {
  const state = entry.bound ? (mockSharersOnline ? "online" : "offline") : "unused";
  return {
    id: entry.id,
    kind: entry.kind,
    name: entry.name,
    state,
    visitors: entry.bound ? 1 : 0,
  };
}

function shareInfo(share: (typeof mockShares)[number]) {
  return {
    id: share.id,
    path: share.path,
    status: lookup(share.path)?.dir ? "ok" : "missing",
    access: share.access.map(accessView),
  };
}

function shareSummary(share: (typeof mockShares)[number]) {
  return {
    id: share.id,
    path: share.path,
    name: share.path.slice(share.path.lastIndexOf("/") + 1),
    status: lookup(share.path)?.dir ? "ok" : "missing",
  };
}

function receivedStatus() {
  if (!mockSharersOnline) return "offline";
  return mockNoAccess ? "no-access" : "online";
}

function receivedView(item: (typeof mockReceived)[number]) {
  return {
    id: item.id,
    folder: item.folder,
    sharer: item.sharer,
    status: receivedStatus(),
  };
}

const SHARE_COMMANDS: Record<string, (args: Args) => unknown> = {
  share_identity: () => ({ name: mockShareName }),
  set_share_name: (a) => {
    const name = str(a, "name").trim();
    mockShareName = name || null;
  },
  share_folder: (a) => {
    const path = str(a, "path");
    if (!lookup(path)?.dir) throw "Only folders on this computer can be shared.";
    const existing = mockShares.find((share) => share.path === path);
    if (existing) return shareInfo(existing);
    const share = { id: `share-${++mockShareSeq}`, path, access: [] };
    mockShares.push(share);
    return shareInfo(share);
  },
  share_info: (a) => {
    const share = mockShares.find((item) => item.path === str(a, "path"));
    return share ? shareInfo(share) : null;
  },
  list_shares: () => mockShares.map(shareSummary),
  create_link: (a) => {
    const share = mockShares.find((item) => item.id === str(a, "share"));
    if (!share) throw "This folder is no longer shared.";
    const kind = str(a, "kind") === "person" ? "person" : "anyone";
    const entry = {
      id: `entry-${++mockLinkSeq}`,
      kind: kind as "anyone" | "person",
      name: kind === "person" ? (a.person as string) ?? "Guest" : "",
      link: "",
      bound: false,
    };
    entry.link = `fbshare1:own-${share.id}-${entry.id}`;
    share.access.push(entry);
    return { link: entry.link, entry: accessView(entry) };
  },
  share_link: (a) => {
    const share = mockShares.find((item) => item.id === str(a, "share"));
    const entry = share?.access.find((item) => item.id === str(a, "entry"));
    if (!entry) throw "This link no longer exists.";
    return entry.link;
  },
  remove_access: (a) => {
    const share = mockShares.find((item) => item.id === str(a, "share"));
    if (share) share.access = share.access.filter((item) => item.id !== str(a, "entry"));
  },
  stop_sharing: (a) => {
    mockShares = mockShares.filter((item) => item.id !== str(a, "share"));
  },
  add_received: (a) => {
    const link = str(a, "link").trim();
    if (!link.startsWith("fbshare1:")) throw "This isn't a share link.";
    if (link.startsWith("fbshare1:own")) throw "This is your own shared folder.";
    if (mockReceived.some((item) => item.link === link)) {
      throw "This folder is already in Network.";
    }
    const item = {
      id: `r${mockReceived.length + 1}`,
      link,
      folder: "Photos",
      sharer: "Ahmed",
    };
    mockReceived.push(item);
    return receivedView(item);
  },
  list_received: () => mockReceived.map(receivedView),
  remove_received: (a) => {
    mockReceived = mockReceived.filter((item) => item.id !== str(a, "id"));
  },
  share_open: (a) => {
    const path = str(a, "path");
    const name = path.slice(path.lastIndexOf("/") + 1);
    if (!name || name === path) throw "Only files can be opened.";
    return `/tmp/opencode/mock-share/${decodeURIComponent(name)}`;
  },
};

(globalThis as Record<string, unknown>).__mockShare = {
  setOnline(online: boolean) {
    mockSharersOnline = online;
  },
  setNoAccess(value: boolean) {
    mockNoAccess = value;
  },
  change(_path: string) {},
  reset() {
    mockShareName = null;
    mockShares = [];
    mockReceived = [];
    mockShareSeq = 0;
    mockLinkSeq = 0;
    mockSharersOnline = true;
    mockNoAccess = false;
  },
};

function driveMakeDir(path: string): string {
  const { parent, name } = driveTarget(path);
  if (driveChildren(parent.id, name).length) throw "A folder with that name already exists.";
  const item = addDriveItem({ id: `drive-${++driveSeq}`, name, parent: parent.id, isDir: true });
  driveLog.push(`create ${name}`);
  return drivePath(item.id);
}

function driveRename(from: string, to: string): string {
  const item = driveItemAt(from);
  if (!item) throw `“${from}” no longer exists`;
  const { parent, name } = driveTarget(to);
  if (driveChildren(parent.id, name).some((other) => other.id !== item.id)) {
    throw "A file or folder with that name already exists.";
  }
  item.name = name;
  return drivePath(item.id);
}

function driveCopyTree(item: DriveItem, parent: string, name = item.name): DriveItem {
  const copy = addDriveItem({ ...item, id: `drive-${++driveSeq}`, parent, name, trashed: false });
  for (const child of driveChildren(item.id)) driveCopyTree(child, copy.id);
  return copy;
}

/** The existing item a transfer would replace, after the shared refusals. */
function driveClash(parentId: string, name: string, overwrite: boolean, sourceId?: string): DriveItem | undefined {
  const existing = driveChildren(parentId, name).filter((other) => other.id !== sourceId);
  if (existing.length && !overwrite) throw `Target already exists: ${name}`;
  if (existing.length > 1) {
    throw `Several items in this folder are named “${name}”. Rename or remove the extra ones in Google Drive first.`;
  }
  return existing[0];
}

function driveWithin(from: string, to: string, overwrite: boolean, move: boolean): string {
  const item = driveItemAt(from);
  if (!item) throw `“${from}” no longer exists`;
  const { parent, name } = driveTarget(to);
  if (parseDrive(to).ids.includes(item.id)) throw "Cannot copy or move a folder into itself";
  if (item.parent === parent.id && item.name === name) throw "The source and destination are the same";
  const old = driveClash(parent.id, name, overwrite, item.id);
  if (old) old.trashed = true;
  if (!move) return drivePath(driveCopyTree(item, parent.id, name).id);
  item.parent = parent.id;
  item.name = name;
  return drivePath(item.id);
}

function driveUpload(from: string, to: string, overwrite: boolean, move: boolean): string {
  const [fromParent, fromName] = parentAndName(from);
  const node = fromParent?.dir ? fromParent.children[fromName] : undefined;
  if (!node || !fromParent?.dir) throw `“${from}” no longer exists`;
  const { parent, name } = driveTarget(to);
  const old = driveClash(parent.id, name, overwrite);
  if (old) old.trashed = true;
  const add = (n: Node, parentId: string, itemName: string): DriveItem => {
    const item = addDriveItem({ id: `drive-${++driveSeq}`, name: itemName, parent: parentId, isDir: n.dir, size: n.dir ? 0 : n.size });
    if (n.dir) for (const [child, sub] of Object.entries(n.children)) add(sub, item.id, child);
    return item;
  };
  const uploaded = add(node, parent.id, name);
  driveLog.push(`upload ${name}`);
  if (move) trash(fromParent, fromName, from);
  return drivePath(uploaded.id);
}

function driveDownload(from: string, to: string, overwrite: boolean, move: boolean): string {
  const item = driveItemAt(from);
  if (!item) throw `“${from}” no longer exists`;
  if (item.doc) throw `“${item.name}” is a Google Docs file and can only be copied within Google Drive.`;
  if (["", ".", ".."].includes(to.split("/").pop() ?? "")) throw `“${to}” is not a valid name for a local item.`;
  const [toParent, toName] = parentAndName(to);
  if (!toParent?.dir) throw "Failed to copy: No such file or directory";
  if (toParent.children[toName]) {
    if (!overwrite) throw `Target already exists: ${to}`;
    trash(toParent, toName);
  }
  // Like the backend: Google Docs are left out and named in an error after
  // the rest is copied, and a move then keeps its source.
  const skipped: string[] = [];
  const build = (d: DriveItem): Node => {
    if (!d.isDir) return { dir: false, size: d.size, modified: now };
    const children = driveChildren(d.id).filter((c) => !(c.doc && skipped.push(c.name)));
    return { dir: true, modified: now, children: Object.fromEntries(children.map((c) => [c.name, build(c)])) };
  };
  toParent.children[toName] = build(item);
  driveLog.push(`download ${item.name}`);
  if (skipped.length) throw `“${skipped[0]}” is a Google Docs file and can only be copied within Google Drive.`;
  if (move) {
    item.trashed = true;
    driveLog.push(`trash ${item.name}`);
  }
  return to;
}

/** copy_entry / move_entry when either side is in Drive. */
function driveTransfer(args: Args, move: boolean): string {
  const from = str(args, "from"), to = str(args, "to");
  const overwrite = !!args.overwrite;
  const [fromCloud, toCloud] = [from.startsWith("gdrive://"), to.startsWith("gdrive://")];
  if (fromCloud && toCloud) return driveWithin(from, to, overwrite, move);
  return fromCloud ? driveDownload(from, to, overwrite, move) : driveUpload(from, to, overwrite, move);
}

function driveTrash(path: string, trashed: boolean): string {
  const item = driveItems.get(parseDrive(path).ids.slice(-1)[0]);
  if (!item) throw `“${path}” no longer exists`;
  item.trashed = trashed;
  driveLog.push(`${trashed ? "trash" : "restore"} ${item.name}`);
  return path;
}

function driveProperties(path: string) {
  const entry = driveNode(path);
  if (!entry) throw "Could not read properties: Google Drive item no longer exists.";
  return {
    name: entry.name,
    path,
    parent: path.replace(/\/[^/]*$/, ""),
    is_dir: entry.is_dir,
    is_symlink: false,
    symlink_target: null,
    size: entry.size,
    item_count: null,
    modified: entry.modified,
    accessed: entry.modified,
    created: 0,
    mode: "",
    octal: "",
    owner: "me@example.com",
    group: "",
    others: "",
  };
}

// `?demo` hides the entries that only exist for testing (used for README screenshots).
const DEMO = new URLSearchParams(location.search).has("demo");
const TEST_ONLY = new Set(["/Users/me/a-very-long-folder-name-that-needs-to-wrap-onto-two-lines", "/big", "/Users/me/Pictures/Camera"]);

function list(path: string, showHidden: boolean): Entry[] {
  const node = lookup(path);
  if (!node) throw `Failed to read directory: No such file or directory (os error 2)`;
  if (!node.dir) throw `Failed to read directory: Not a directory (os error 20)`;
  if (path === "/System") throw `Failed to read directory: Operation not permitted (os error 1)`;
  const base = path.replace(/\/$/, "");
  return Object.entries(node.children)
    .filter(([name]) => name !== ".versions")
    .filter(([name]) => showHidden || !name.startsWith("."))
    .filter(([name]) => !DEMO || !TEST_ONLY.has(`${base}/${name}`))
    .map(([name, child]) => ({
      name,
      path: `${base}/${name}`,
      is_dir: child.dir,
      size: child.dir ? 96 : child.size,
      modified: child.modified,
      is_hidden: name.startsWith("."),
      is_symlink: !!child.link,
    }));
}

function parentAndName(path: string): [Node | undefined, string] {
  const i = path.lastIndexOf("/");
  return [lookup(path.slice(0, i) || "/"), path.slice(i + 1)];
}

const TRASH = "/Users/me/.Trash";
/** Where the mock reports opened read-only copies. */
const OPENED = "/Users/me/.cache/opened-versions";
/** Path in the Trash → original path and when it was trashed (the real app's trash_log.json). */
const trashLog = new Map<string, { origin: string; time: number }>([
  [`${TRASH}/old.txt`, { origin: "/Users/me/Documents/old.txt", time: now - 9 * day }],
]);

/** Move `name` out of `parent` into the Trash; returns its path there. */
function trash(parent: Node & { dir: true }, name: string, origin?: string): string {
  const bin = lookup(TRASH);
  if (!bin?.dir) throw "no trash";
  let target = name;
  for (let n = 2; bin.children[target]; n++) target = `${name} ${n}`;
  bin.children[target] = parent.children[name];
  delete parent.children[name];
  const trashed = `${TRASH}/${target}`;
  if (origin) trashLog.set(trashed, { origin, time: Math.floor(Date.now() / 1000) });
  return trashed;
}

/** Make the folders on the way to `path`, like the backend's restore. */
function ensureDir(path: string): Node & { dir: true } {
  let node = lookup("/") as Node & { dir: true };
  for (const part of path.split("/").filter(Boolean)) {
    node.children[part] ??= d({}, 0);
    const next = node.children[part];
    if (!next.dir) throw `“${part}” is not a folder`;
    node = next;
  }
  return node;
}

/** Path → last used time, standing in for Spotlight plus the app's own list. */
const recentUse = new Map<string, number>([
  ["/Users/me/Documents/Quarterly report.pdf", now - 20 * 60],
  ["/Users/me/Downloads/invoice-0423.pdf", now - 2 * 3600],
  ["/Users/me/Documents/Notes.md", now - 5 * 3600],
  ["/Users/me/Pictures/IMG_20 2.heic", now - day - 3 * 3600],
  ["/Users/me/Pictures/Screenshots/Screenshot 1.png", now - 2 * day],
  ["/Users/me/main.rs", now - 3 * day],
  ["/Users/me/Documents/Budget 2026.xlsx", now - 6 * day],
  ["/Users/me/Music/Track 3.flac", now - 12 * day],
  ["/Users/me/.config/settings.json", now - 4 * day],
]);

function recent(showHidden: boolean): (Entry & { used: number })[] {
  const out: (Entry & { used: number })[] = [];
  for (const [path, used] of recentUse) {
    const [parent, name] = parentAndName(path);
    if (!parent?.dir || !parent.children[name]) continue;
    if (!showHidden && path.slice("/Users/me/".length).split("/").some((p) => p.startsWith("."))) continue;
    const entry = list(path.slice(0, path.lastIndexOf("/")), true).find((e) => e.name === name);
    if (entry) out.push({ ...entry, used });
  }
  return out.sort((a, b) => b.used - a.used);
}

let pins: string[] = ["/Users/me/Documents/Projects"];
let starred: string[] = ["/Users/me/Documents/Quarterly report.pdf", "/Users/me/Documents/Projects", "/Users/me/Pictures/IMG_20 3.heic"];

function entryAt(path: string): Entry | undefined {
  const i = path.lastIndexOf("/");
  try {
    return list(path.slice(0, i) || "/", true).find((e) => e.name === path.slice(i + 1));
  } catch {
    return undefined;
  }
}

/** Bytes of every file below a node, for the Properties dialog. */
function totalSize(node: Node): number {
  return node.dir ? Object.values(node.children).reduce((sum, child) => sum + totalSize(child), 0) : node.size;
}

const flags: Record<string, boolean> = {};
let mockWalks = 0;

/** A made-up picture for image/video/PDF files, so thumbnails can be previewed. */
async function mockThumbnail(path: string, size: number): Promise<ArrayBuffer> {
  if (!/\.(png|jpe?g|heic|gif|webp|mov|mp4|pdf)$/i.test(path)) return new ArrayBuffer(0);
  await new Promise((r) => setTimeout(r, 150));
  const portrait = /\.pdf$/i.test(path);
  const [w, h] = portrait ? [Math.round(size * 0.75), size] : [size, Math.round(size * 0.66)];
  const canvas = new OffscreenCanvas(w, h);
  const ctx = canvas.getContext("2d")!;
  const seed = [...path].reduce((a, c) => a + c.charCodeAt(0), 0);
  if (portrait) drawDocument(ctx, w, h, seed);
  else drawPhoto(ctx, w, h, seed);
  // Photos/Camera stands in for real photos: grain makes the PNG about as
  // large as a real photo thumbnail, for checking the cache's memory limit.
  if (!portrait && path.includes("/Camera/")) addGrain(ctx, w, h, seed);
  return (await canvas.convertToBlob({ type: "image/png" })).arrayBuffer();
}

/** A tiny pseudo-random sequence, so each file always gets the same picture. */
const nextSeed = (seed: number) => (seed * 9301 + 49297) % 233280;

function drawDocument(ctx: OffscreenCanvasRenderingContext2D, w: number, h: number, seed: number): void {
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, w, h);
  ctx.fillStyle = "#c01c28";
  ctx.fillRect(w * 0.1, h * 0.08, w * 0.8, h * 0.1);
  ctx.fillStyle = "#c0bfbc";
  for (let y = 0.25; y < 0.9; y += 0.06) {
    seed = nextSeed(seed);
    ctx.fillRect(w * 0.1, h * y, w * (0.5 + (seed / 233280) * 0.3), h * 0.025);
  }
}

function drawPhoto(ctx: OffscreenCanvasRenderingContext2D, w: number, h: number, seed: number): void {
  const hue = (seed * 47) % 360;
  const g = ctx.createLinearGradient(0, 0, 0, h);
  g.addColorStop(0, `hsl(${hue} 70% 70%)`);
  g.addColorStop(1, `hsl(${(hue + 40) % 360} 60% 45%)`);
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, w, h);
  ctx.fillStyle = `hsl(${(hue + 180) % 360} 50% 30% / .8)`;
  ctx.beginPath();
  ctx.moveTo(0, h);
  ctx.lineTo(w * 0.35, h * 0.45);
  ctx.lineTo(w * 0.6, h * 0.75);
  ctx.lineTo(w * 0.8, h * 0.55);
  ctx.lineTo(w, h);
  ctx.fill();
  ctx.fillStyle = "#f6d32d";
  ctx.beginPath();
  ctx.arc(w * 0.75, h * 0.25, h * 0.1, 0, Math.PI * 2);
  ctx.fill();
}

function addGrain(ctx: OffscreenCanvasRenderingContext2D, w: number, h: number, seed: number): void {
  const img = ctx.getImageData(0, 0, w, h);
  for (let i = 0; i < img.data.length; i += 4) {
    seed = nextSeed(seed);
    const n = (seed / 233280 - 0.5) * 60;
    img.data[i] += n;
    img.data[i + 1] += n;
    img.data[i + 2] += n;
  }
  ctx.putImageData(img, 0, 0);
}

/** Delay per call so the preview feels like a real backend; tests set 0. */
const DELAY = ((globalThis as Record<string, unknown>).__MOCK_DELAY as number | undefined) ?? 30;

type Args = Record<string, unknown>;
const str = (args: Args, key: string) => args[key] as string;

function fileProperties(path: string) {
  const node = lookup(path);
  if (!node) throw "Could not read properties: No such file or directory (os error 2)";
  const i = path.lastIndexOf("/");
  const isDir = node.dir;
  return {
    name: path.slice(i + 1) || path,
    path,
    parent: path.slice(0, i) || "/",
    is_dir: isDir,
    is_symlink: !!node.link,
    symlink_target: node.link ? "/Users/me/Documents" : null,
    size: isDir ? 96 : node.size,
    item_count: isDir ? Object.keys(node.children).filter((n) => n !== ".versions").length : null,
    modified: node.modified,
    accessed: node.modified,
    created: node.modified - 100 * day,
    mode: isDir ? "drwxr-xr-x" : node.link ? "lrwxrwxrwx" : "-rw-r--r--",
    octal: isDir ? "755" : "644",
    owner: isDir ? "Create and delete files" : "Read and write",
    group: isDir ? "Access files" : "Read-only",
    others: isDir ? "Access files" : "Read-only",
  };
}

function makeDir(path: string): void {
  const [parent, name] = parentAndName(path);
  if (!parent?.dir) throw "Failed to create directory: No such file or directory";
  if (parent.children[name]) throw `Path already exists: ${path}`;
  parent.children[name] = d({}, 0);
}

/** The same refusals as the backend's copy/move, in the same order. */
function checkTransfer(from: string, to: string): { node: Node; fromParent: Node & { dir: true }; toParent: Node & { dir: true } } {
  const [fromParent, fromName] = parentAndName(from);
  const [toParent] = parentAndName(to);
  const node = fromParent?.dir ? fromParent.children[fromName] : undefined;
  if (!node || !fromParent?.dir) throw `“${from}” no longer exists`;
  if (!toParent?.dir) throw "Failed to copy: No such file or directory";
  if (from === to) throw "The source and destination are the same";
  if (to.startsWith(from + "/")) throw "Cannot copy or move a folder into itself";
  return { node, fromParent, toParent };
}

function transfer(args: Args, move: boolean): void {
  const from = str(args, "from"), to = str(args, "to");
  const { node, fromParent, toParent } = checkTransfer(from, to);
  const toName = to.slice(to.lastIndexOf("/") + 1);
  if (toParent.children[toName]) {
    if (!args.overwrite) throw `Target already exists: ${to}`;
    trash(toParent, toName);
  }
  toParent.children[toName] = move ? node : structuredClone(node);
  if (move) delete fromParent.children[from.slice(from.lastIndexOf("/") + 1)];
}

function trashEntry(path: string): string {
  const [parent, name] = parentAndName(path);
  if (!parent?.dir || !parent.children[name]) throw `“${path}” no longer exists`;
  return trash(parent, name, path);
}

function restoreEntry(path: string): string {
  const logged = trashLog.get(path);
  if (!logged) throw "The original location is unknown";
  const [bin, name] = parentAndName(path);
  if (!bin?.dir || !bin.children[name]) throw `“${path}” is no longer in the Trash`;
  const [, originName] = parentAndName(logged.origin);
  const dir = ensureDir(logged.origin.slice(0, logged.origin.lastIndexOf("/")));
  if (dir.children[originName]) throw `An item named “${originName}” already exists in “${logged.origin.split("/").at(-2)}”`;
  dir.children[originName] = bin.children[name];
  delete bin.children[name];
  trashLog.delete(path);
  return logged.origin;
}

function deletePermanently(path: string): void {
  if (!path.startsWith(TRASH + "/")) throw "Only items in the Trash can be deleted permanently";
  const [parent, name] = parentAndName(path);
  if (!parent?.dir || !parent.children[name]) throw `“${path}” no longer exists`;
  delete parent.children[name];
}

function emptyTrash(): void {
  const bin = lookup(TRASH);
  if (bin?.dir) bin.children = {};
  trashLog.clear();
}

function renameEntry(from: string, to: string): void {
  const [fromParent, fromName] = parentAndName(from);
  const [toParent, toName] = parentAndName(to);
  if (!fromParent?.dir || !toParent?.dir || !fromParent.children[fromName]) throw "Failed to rename: No such file or directory";
  if (toParent.children[toName]) throw `Target already exists: ${to}`;
  toParent.children[toName] = fromParent.children[fromName];
  delete fromParent.children[fromName];
}

function folderSummary(path: string, showHidden: boolean): { count: number; versioned: boolean } | null {
  try {
    const node = lookup(path);
    const versioned = !!(node?.dir && node.children[".versions"]) && versionedRoots.has(path);
    return { count: list(path, showHidden).length, versioned };
  } catch {
    return null;
  }
}

function pathInfo(path: string) {
  const node = lookup(path);
  return node ? { is_dir: node.dir, device: path.startsWith("/Volumes/Backup Drive") ? 2 : 1 } : null;
}

function trashInfo(path: string) {
  const logged = trashLog.get(path);
  return { origin: logged?.origin ?? null, trashed: logged?.time ?? now - 2 * day };
}

// ---------------------------------------------------------------------------
// Keep Versions
// ---------------------------------------------------------------------------

/** Folders the mock reports as versioned. */
const versionedRoots = new Set<string>(["/Users/me/Documents/Versioned"]);

/** Canned version lists for the files in Documents/Versioned. */
function mockVersions(path: string) {
  const v = (id: number, size: number, ago: number, current = false) =>
    ({ id, size, mtime: now - ago, recorded_at: now - ago, deleted: false, current });
  if (path.endsWith("/notes.txt")) {
    return [
      v(3, 2_100, 3_600, true),
      v(2, 2_000, 7_200),
      v(1, 1_700, day),
      { id: 0, size: 0, mtime: 0, recorded_at: now - 2 * day, deleted: true, current: false },
    ];
  }
  if (path.endsWith("/image.png")) return [v(1, 120_000, day, true)];
  if (path.endsWith("/large.txt")) return [v(2, 2_000_000, 60, true), v(1, 1_900_000, 120)];
  return [];
}

function versionedRoot(path: string): string | null {
  return [...versionedRoots].find((r) => path === r || path.startsWith(r + "/")) ?? null;
}

function makeVersionedDir(path: string): void {
  makeDir(path);
  const [parent, name] = parentAndName(path);
  (parent as Node & { dir: true }).children[name] = d({ ".versions": d({ "index.sqlite": f(12_000, 0) }) }, 0);
  versionedRoots.add(path);
}

function versionDiff(path: string, version: number) {
  if (path.endsWith("/image.png")) return { kind: "binary" };
  if (path.endsWith("/large.txt")) return { kind: "too_large" };
  if (version === 0) return { kind: "deleted" };
  if (version >= 3) return { kind: "same" };
  return {
    kind: "text",
    lines: [
      { tag: " ", text: "# Notes" },
      { tag: "-", text: version === 1 ? "first draft" : "old line" },
      { tag: "+", text: "new line" },
    ],
    truncated: false,
  };
}

/** Put a stand-in for a restored file at `dir/name`; refuses to overwrite. */
function placeRestored(path: string, name: string, size: number, clash: string): string {
  const [parent] = parentAndName(path);
  if (!parent?.dir) throw "Could not restore: missing folder";
  if (parent.children[name]) throw clash;
  parent.children[name] = f(size, 0);
  return `${path.slice(0, path.lastIndexOf("/"))}/${name}`;
}

function replaceWithVersion(path: string): void {
  const [parent, name] = parentAndName(path);
  if (!parent?.dir || !parent.children[name]) throw "The file is not in the folder any more";
  parent.children[name] = f(1_700, 0);
}

function clearHistory(root: string): void {
  const node = lookup(root);
  if (node?.dir) node.children[".versions"] = d({ "index.sqlite": f(12_000, 0) }, 0);
}

function stopKeepingVersions(root: string): void {
  const node = lookup(root);
  if (node?.dir) delete node.children[".versions"];
  versionedRoots.delete(root);
}

const VERSION_COMMANDS: Record<string, (args: Args) => unknown> = {
  versioned_root: (a) => versionedRoot(str(a, "path")),
  make_versioned_dir: (a) => makeVersionedDir(str(a, "path")),
  list_versions: (a) => mockVersions(str(a, "path")),
  version_count: (a) => mockVersions(str(a, "path")).length,
  version_diff: (a) => versionDiff(str(a, "path"), a.version as number),
  restore_version_copy: (a) =>
    placeRestored(str(a, "path"), str(a, "name"), 1_700, `An item named “${str(a, "name")}” already exists`),
  open_version: (a) => `${OPENED}/${str(a, "name")}`,
  replace_with_version: (a) => replaceWithVersion(str(a, "path")),
  deleted_files: (a) => [{ path: `${str(a, "dir")}/draft.md`, deleted_at: now - 3 * 3600, size: 320 }],
  restore_deleted: (a) => {
    const path = str(a, "path");
    return placeRestored(path, path.slice(path.lastIndexOf("/") + 1), 320, "Something is already at that path");
  },
  open_deleted: (a) => `${OPENED}/${str(a, "name")}`,
  history_info: () => ({ bytes: 12_400_000, versions: 214, files: 37 }),
  clear_history: (a) => clearHistory(str(a, "root")),
  stop_keeping_versions: (a) => stopKeepingVersions(str(a, "root")),
};

/** Offline stand-in for the Google Drive account commands. */
const DRIVE_SCOPE = "https://www.googleapis.com/auth/drive";
interface MockDriveAccount {
  id: string;
  email: string;
  name: string;
  root_id: string;
  root_name: string;
  root_path: string;
  scopes: string[];
  connected_at: number;
}
let driveAccount: MockDriveAccount | null = null;
/** The user's OAuth client id; the mock checks the file's shape like the backend. */
let driveClientId: string | null = null;

function parseDriveClient(json: string): string {
  const notClient =
    "This is not an OAuth client file. Download the JSON from the client's page in Google Cloud.";
  let file: { installed?: { client_id?: string; client_secret?: string }; web?: unknown };
  try {
    file = JSON.parse(json);
  } catch {
    throw notClient;
  }
  if (!file.installed) {
    throw file.web ? "This is a web application client. Create a client of type Desktop app." : notClient;
  }
  const id = file.installed.client_id?.trim() ?? "";
  if (!id.endsWith(".apps.googleusercontent.com")) throw "The client file has no valid client ID.";
  if (!file.installed.client_secret?.trim()) {
    throw "The client file has no client secret. Download it again from Google Cloud.";
  }
  return id;
}

function refuseWhileConnected(): void {
  if (driveAccount) throw "Disconnect Google Drive first.";
}
const driveFlows = new Set<string>();
let driveFlowSeq = 0;

const DRIVE_COMMANDS: Record<string, (args: Args) => unknown> = {
  drive_accounts: () => (driveAccount ? [{ ...driveAccount }] : []),
  drive_client_status: () => ({ configured: driveClientId !== null, client_id: driveClientId }),
  drive_client_set: (a) => {
    refuseWhileConnected();
    driveClientId = parseDriveClient(str(a, "json"));
    return { configured: true, client_id: driveClientId };
  },
  drive_client_clear: () => {
    refuseWhileConnected();
    driveClientId = null;
  },
  drive_connect_start: () => {
    if (!driveClientId) {
      throw "Google Drive needs your own OAuth client. Add one in the Connect dialog.";
    }
    const flow_id = `flow-${++driveFlowSeq}`;
    driveFlows.add(flow_id);
    return {
      flow_id,
      auth_url: "https://accounts.google.com/o/oauth2/v2/auth?mock=1",
    };
  },
  drive_connect_finish: (a) => {
    if (!driveFlows.delete(str(a, "flowId"))) {
      throw "This sign-in attempt is no longer active.";
    }
    driveAccount = {
      id: "mock-account",
      email: "me@example.com",
      name: "Me",
      root_id: "mock-root",
      root_name: "My Drive",
      root_path: DRIVE_ROOT,
      scopes: [DRIVE_SCOPE],
      connected_at: now,
    };
    return { ...driveAccount };
  },
  drive_connect_cancel: (a) => void driveFlows.delete(str(a, "flowId")),
  drive_disconnect: () => {
    driveAccount = null;
  },
  drive_open: (a) => {
    const entry = driveNode(str(a, "path"));
    if (entry?.cloud?.web_url) return { kind: "url", url: entry.cloud.web_url };
    return { kind: "file", path: `/tmp/opencode/mock-drive/${entry?.name ?? "file"}` };
  },
};

/** Every command the app calls, by name. */
const COMMANDS: Record<string, (args: Args) => unknown> = {
  list_dir: (a) => {
    const path = str(a, "path");
    if (path.startsWith("share://")) return listShare(path);
    return path.startsWith("gdrive://") ? listDrive(path) : list(path, a.showHidden as boolean);
  },
  item_counts: (a) =>
    (a.paths as string[]).map((path) =>
      path.startsWith("gdrive://") || path.startsWith("share://")
        ? null
        : folderSummary(path, a.showHidden as boolean),
    ),
  file_properties: (a) => {
    const path = str(a, "path");
    if (path.startsWith("share://")) return shareProperties(path);
    return path.startsWith("gdrive://") ? driveProperties(path) : fileProperties(path);
  },
  new_folder_walk: () => ++mockWalks,
  cancel_folder_size: () => undefined,
  folder_size: (a) => {
    const node = lookup(str(a, "path"));
    return node ? totalSize(node) : 0;
  },
  make_dir: (a) => {
    const path = str(a, "path");
    if (path.startsWith("gdrive://")) return driveMakeDir(path);
    makeDir(path);
    return path;
  },
  load_pins: () => [...pins],
  save_pins: (a) => void (pins = [...(a.paths as string[])]),
  get_flag: (a) => flags[str(a, "name")] ?? false,
  set_flag: (a) => void (flags[str(a, "name")] = a.value as boolean),
  recent_files: (a) => recent(a.showHidden as boolean),
  add_recent: (a) => void recentUse.set(str(a, "path"), Math.floor(Date.now() / 1000)),
  forget_recent: (a) => void (a.paths as string[]).forEach((p) => recentUse.delete(p)),
  load_starred: () => [...starred],
  save_starred: (a) => void (starred = [...(a.paths as string[])]),
  starred_files: (a) => starred.map(entryAt).filter((e): e is Entry => !!e && (!!a.showHidden || !e.is_hidden)),
  has_full_disk_access: () => false,
  // The preview pretends to be macOS, so it lists /Volumes the way the real
  // backend would on a Mac.
  volumes: () => [
    { name: "Backup Drive", path: "/Volumes/Backup Drive" },
    { name: "Macintosh HD", path: "/Volumes/Macintosh HD" },
  ],
  trash_dir: () => TRASH,
  thumbnail: (a) => {
    const path = str(a, "path");
    return path.startsWith("gdrive://") || path.startsWith("share://")
      ? Promise.resolve(new ArrayBuffer(0))
      : mockThumbnail(path, a.size as number);
  },
  path_info: (a) =>
    (a.paths as string[]).map((path) => {
      if (path.startsWith("share://")) {
        const entry = shareNode(path);
        return entry ? { is_dir: entry.is_dir, device: 0 } : null;
      }
      if (!path.startsWith("gdrive://")) return pathInfo(path);
      const entry = driveNode(path);
      return entry ? { is_dir: entry.is_dir, device: 0 } : null;
    }),
  path_exists: (a) => {
    const path = str(a, "path");
    if (path.startsWith("share://")) return !!shareNode(path);
    return path.startsWith("gdrive://") ? drivePathExists(path) : lookup(path) !== undefined;
  },
  copy_entry: (a) => (isDriveTransfer(a) ? driveTransfer(a, false) : (transfer(a, false), str(a, "to"))),
  move_entry: (a) => (isDriveTransfer(a) ? driveTransfer(a, true) : (transfer(a, true), str(a, "to"))),
  trash_entry: (a) => {
    const path = str(a, "path");
    return path.startsWith("gdrive://") ? driveTrash(path, true) : trashEntry(path);
  },
  trash_info: (a) => (a.paths as string[]).map(trashInfo),
  restore_entry: (a) => {
    const path = str(a, "path");
    return path.startsWith("gdrive://") ? driveTrash(path, false) : restoreEntry(path);
  },
  delete_permanently: (a) => deletePermanently(str(a, "path")),
  empty_trash: () => emptyTrash(),
  rename_entry: (a) => {
    const from = str(a, "from"), to = str(a, "to");
    if (from.startsWith("gdrive://") || to.startsWith("gdrive://")) return driveRename(from, to);
    renameEntry(from, to);
    return to;
  },
  ...SHARE_COMMANDS,
  ...DRIVE_COMMANDS,
  ...VERSION_COMMANDS,
};

function isDriveTransfer(a: Args): boolean {
  return str(a, "from").startsWith("gdrive://") || str(a, "to").startsWith("gdrive://");
}

export async function invoke<T>(cmd: string, args: Args = {}): Promise<T> {
  await new Promise((r) => setTimeout(r, DELAY));
  const command = COMMANDS[cmd];
  if (!command) throw `mock: unknown command ${cmd}`;
  return (await command(args)) as T;
}

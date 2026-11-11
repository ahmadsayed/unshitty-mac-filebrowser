import { encodeCloudName, isArbeskAsset, isUnder, placeOf, type Place } from "../../src/paths";
import { fold } from "../../src/search";
import { BUILT_IN_CAPABILITIES } from "../../src/capabilities";
import { sample3mf, sampleGlb, sampleModelBytes } from "./models";
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
  /** An item's other files (an Arbesk asset's representations), when it has some. */
  representations?: { count: number; stale: number; formats: string[] };
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
        Projects: d({
          "unshitty-mac-filebrowser": d({ "README.md": f(2_200, 60) }),
          website: d(),
          // 3D models for the Space preview; the bytes come from ./models.
          Prints: d({
            "bracket.stl": f(3_412, 2 * day),
            "bracket-binary.stl": f(684, 2 * day),
            "box.3mf": f(1_102, 3 * day),
            "lamp.glb": f(1_620, 4 * day),
            "lamp.gltf": f(1_150, 4 * day),
            "lamp.bin": f(168, 4 * day),
            "scan.stl": f(900_000_000, 9 * day),
          }),
        }),
        Versioned: d({
          "notes.txt": f(2_100, 2 * 3600),
          "image.png": f(120_000, day),
          "large.txt": f(2_000_000, day),
          "archive.zip": f(5_000_000, day),
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
      Templates: d({ "Letter.odt": f(0, 10 * day), "Budget.ods": f(0, 10 * day) }),
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

/** The account key in a Drive path: every mock account shows the same tree. */
function driveKey(path: string): string {
  return path.slice("gdrive://".length).split("/")[0] || "mock";
}

function drivePath(id: string, key = "mock"): string {
  const parts: string[] = [];
  for (let item = driveItems.get(id); item; item = item.parent ? driveItems.get(item.parent) : undefined) {
    parts.unshift(`${item.id}~${encodeCloudName(item.name)}`);
  }
  return `gdrive://${key}/${parts.join("/")}`;
}

/** The ids along a Drive path and, for a pending `~name` tail, that name. */
function parseDrive(path: string): { ids: string[]; pending?: string } {
  const segments = path.slice(`gdrive://${driveKey(path)}/`.length).split("/").filter(Boolean);
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

function driveEntry(item: DriveItem, key = "mock"): Entry {
  return {
    name: item.name,
    path: drivePath(item.id, key),
    is_dir: item.isDir,
    size: item.size,
    modified: now - day,
    is_hidden: false,
    is_symlink: false,
    cloud: {
      account: key,
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
  return driveChildren(folder.id).map((item) => driveEntry(item, driveKey(path)));
}

function driveNode(path: string): Entry | undefined {
  const item = driveItemAt(path);
  return item && driveEntry(item, driveKey(path));
}

function drivePathExists(path: string): boolean {
  const { ids, pending } = parseDrive(path);
  if (pending !== undefined) return driveChildren(ids[ids.length - 1], pending).length > 0;
  return !!driveItemAt(path);
}

// ---------------------------------------------------------------------------
// A fake SSH/SFTP server for the Network preview.
// ---------------------------------------------------------------------------

interface SftpServer {
  id: string;
  label: string;
  url: string;
  host: string;
  port: number;
  user: string;
}

let mockSftpServers: SftpServer[] = [];
let mockSftpNeedsAuth = false;
let mockSftpUnknownHost = false;

/** A server's files by `user@host:<path>`. A connection starts in the user's
 *  home (`/home/<user>`), below the root, as `sftp.rs` starts it. */
const SFTP_FILES: Record<string, Array<{ name: string; isDir: boolean; size: number }>> = {
  "user@host:/": [{ name: "home", isDir: true, size: 0 }],
  "user@host:/home": [{ name: "user", isDir: true, size: 0 }],
  "user@host:/home/user": [
    { name: "Documents", isDir: true, size: 0 },
    { name: "notes.txt", isDir: false, size: 42 },
  ],
  "user@host:/home/user/Documents": [{ name: "hello.txt", isDir: false, size: 12 }],
};

function sftpParts(path: string): { id: string; rest: string } | null {
  if (placeOf(path) !== "sftp") return null;
  const rest = path.slice("sftp://".length);
  const slash = rest.indexOf("/");
  const authority = slash < 0 ? rest : rest.slice(0, slash);
  const tail = slash < 0 ? "/" : rest.slice(slash);
  const [user, hostPort] = authority.split("@");
  const [host, portText] = (hostPort ?? "").split(":");
  const port = portText ? Number(portText) : 22;
  if (!user || !host) return null;
  return { id: `${user}@${host}:${port}`, rest: tail || "/" };
}

function listSftp(path: string): Entry[] {
  const parts = sftpParts(path);
  if (!parts || !mockSftpServers.some((server) => server.id === parts.id)) {
    throw "Connect to this server first.";
  }
  const files = SFTP_FILES[`${parts.id.split(":")[0]}:${parts.rest}`] ?? [];
  const base = path.replace(/\/+$/, "");
  return files.map((file) => ({
    name: file.name,
    path: `${base}/${encodeURIComponent(file.name)}`,
    is_dir: file.isDir,
    size: file.size,
    modified: now - day,
    is_hidden: false,
    is_symlink: false,
  }));
}

// ---------------------------------------------------------------------------
/**
 * A link in the real shape, `share://` + 64 or more base32 characters, so the
 * frontend tells it from a received share's `share://<id>` location. `tag`
 * keeps own links recognisable ("own…") and each link distinct.
 */
function mockLink(tag: string): string {
  const b32 = "abcdefghijklmnopqrstuvwxyz234567";
  const ticket = [...tag.toLowerCase()].map((c) => (b32.includes(c) ? c : b32[c.charCodeAt(0) % 32])).join("");
  return `share://${ticket.padEnd(64, "a")}`;
}

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
  access: Array<{ id: string; kind: "anyone" | "person"; name: string; link: string; bound: boolean; protected: boolean }>;
}> = [];
let mockReceived: Array<{ id: string; link: string; ticket: string; folder: string; sharer: string }> = [];
let receivedHeld = false;
const receivedWaiters: (() => void)[] = [];
let mockShareSeq = 0;
let mockLinkSeq = 0;
let mockSharersOnline = true;
let mockNoAccess = false;
// The sharer dropped after Network last asked: the status still says online,
// but listing fails as if offline.
let mockSharerUnreachable = false;

// Simulate unlocking in memory. Real cryptography is exercised by Rust tests.
const mockProtectedLinks = new Map<string, { ticket: string; passphrase: string }>();
let mockProtectedSeq = 0;

function mockShareTicket(text: string): string {
  let link = text.trim();
  if (link.startsWith("<") && link.endsWith(">")) link = link.slice(1, -1).trim();
  link = link.toLowerCase();
  const ticket = link.startsWith("share://") ? link.slice("share://".length).replace(/\/$/, "") : "";
  if (!ticket || !/^[a-z0-9-]+$/.test(ticket) || ticket.startsWith("bad")) throw "This isn't a share link.";
  if (ticket.startsWith("future")) throw "This link needs a newer version of Files.";
  if (ticket.startsWith("locked") && !mockProtectedLinks.has(ticket)) throw "This isn't a share link.";
  return ticket;
}

function protectMockShare(link: string, passphrase: string): string {
  if (!passphrase) throw "Enter a passphrase.";
  if (new TextEncoder().encode(passphrase).length > 1024) throw "Use a passphrase of at most 1,024 bytes.";
  const ticket = mockShareTicket(link);
  const protectedLink = mockLink(`locked${++mockProtectedSeq}`);
  mockProtectedLinks.set(protectedLink.slice("share://".length), { ticket, passphrase });
  return protectedLink;
}

function unlockMockShare(link: string, passphrase?: string): string {
  const ticket = mockShareTicket(link);
  const protectedLink = mockProtectedLinks.get(ticket);
  if (!protectedLink) return ticket;
  if (passphrase === undefined || passphrase === null) throw "This share link requires a passphrase.";
  if (passphrase !== protectedLink.passphrase) throw "Incorrect passphrase or damaged share link.";
  return protectedLink.ticket;
}

function listShare(path: string): Entry[] {
  if (!mockSharersOnline || mockSharerUnreachable) throw "Ahmed is offline.";
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

function accessView(entry: { id: string; kind: "anyone" | "person"; name: string; bound: boolean; protected: boolean }) {
  const state = entry.bound ? (mockSharersOnline ? "online" : "offline") : "unused";
  return {
    id: entry.id,
    kind: entry.kind,
    name: entry.name,
    state,
    visitors: entry.bound ? 1 : 0,
    protected: entry.protected,
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
    if (!isLocal(path) || !lookup(path)?.dir) throw "Only folders on this computer can be shared.";
    const existing = mockShares.find((share) => share.path === path);
    if (existing) return shareInfo(existing);
    const share = { id: `share-${++mockShareSeq}`, path, access: [] };
    mockShares.push(share);
    return shareInfo(share);
  },
  share_info: (a) => {
    // Shares are folders on this computer: nothing for any other path.
    const share = isLocal(str(a, "path")) && mockShares.find((item) => item.path === str(a, "path"));
    return share ? shareInfo(share) : null;
  },
  list_shares: () => mockShares.map(shareSummary),
  create_link: (a) => {
    const share = mockShares.find((item) => item.id === str(a, "share"));
    if (!share) throw "This folder is no longer shared.";
    const kind = str(a, "kind");
    if (kind !== "anyone" && kind !== "person") throw "Unknown link kind.";
    const person = (a.person as string | undefined)?.trim();
    if (kind === "person" && !person) throw "Enter a name for this person.";
    const passphrase = a.passphrase as string | undefined;
    if (kind === "anyone" && passphrase == null) {
      const existing = share.access.find((entry) => entry.kind === "anyone" && !entry.protected);
      if (existing) return { link: existing.link, entry: accessView(existing) };
    }
    const entry = {
      id: `entry-${++mockLinkSeq}`,
      kind: kind as "anyone" | "person",
      name: kind === "person" ? person! : "Anyone with the link",
      link: "",
      bound: false,
      protected: passphrase != null,
    };
    entry.link = mockLink(`own${share.id}${entry.id}`);
    if (passphrase != null) entry.link = protectMockShare(entry.link, passphrase);
    share.access.push(entry);
    return { link: entry.link, entry: accessView(entry) };
  },
  share_link_requires_passphrase: (a) => mockProtectedLinks.has(mockShareTicket(str(a, "link"))),
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
    const link = str(a, "link");
    const ticket = unlockMockShare(link, a.passphrase as string | undefined);
    if (ticket.startsWith("own")) throw "This is your own shared folder.";
    if (mockReceived.some((item) => item.ticket === ticket)) {
      throw "This folder is already in Network.";
    }
    const item = {
      id: `r${mockReceived.length + 1}`,
      link,
      ticket,
      folder: "Photos",
      sharer: "Ahmed",
    };
    mockReceived.push(item);
    return receivedView(item);
  },
  // Held by tests to stand in for sharers that take a connect timeout to answer.
  list_received: async () => {
    if (receivedHeld) await new Promise<void>((resolve) => receivedWaiters.push(resolve));
    return mockReceived.map(receivedView);
  },
  remove_received: (a) => {
    mockReceived = mockReceived.filter((item) => item.id !== str(a, "id"));
  },
};

/** open_target for a share file: where its downloaded copy lands. */
function shareOpen(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  if (!name || name === path || path.indexOf("/", "share://".length) < 0) throw "Only files can be opened.";
  return `/tmp/opencode/mock-share/${decodeURIComponent(name)}`;
}

(globalThis as Record<string, unknown>).__mockShare = {
  protect: protectMockShare,
  setOnline(online: boolean) {
    mockSharersOnline = online;
  },
  setNoAccess(value: boolean) {
    mockNoAccess = value;
  },
  setUnreachable(value: boolean) {
    mockSharerUnreachable = value;
  },
  holdReceived(hold: boolean) {
    receivedHeld = hold;
    if (!hold) receivedWaiters.splice(0).forEach((resolve) => resolve());
  },
  change(_path: string) {},
  reset() {
    mockProtectedLinks.clear();
    mockProtectedSeq = 0;
    mockShareName = null;
    mockShares = [];
    mockReceived = [];
    mockShareSeq = 0;
    mockLinkSeq = 0;
    mockSharersOnline = true;
    mockNoAccess = false;
    mockSharerUnreachable = false;
  },
};

(globalThis as Record<string, unknown>).__mockSftp = {
  reset() {
    mockSftpServers = [];
    mockSftpNeedsAuth = false;
    mockSftpUnknownHost = false;
  },
  requirePassword() {
    mockSftpNeedsAuth = true;
  },
  unknownHost() {
    mockSftpUnknownHost = true;
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
  const uploaded = uploadNode(node, to, overwrite);
  if (move) trash(fromParent, fromName, from);
  return uploaded;
}

/** `node` uploaded to the new name `to` in Drive; its Drive path. */
function uploadNode(node: Node, to: string, overwrite: boolean): string {
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
    trash(toParent, toName, to);
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
  const [fromCloud, toCloud] = [placeOf(from) === "drive", placeOf(to) === "drive"];
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

/** How many items one search looks at before giving up (the backend's `MAX_SEARCH_SCANNED`). */
const MAX_SEARCH_SCANNED = 100_000;
/** The mock sends matches in batches of this many, so the app sees results stream in. */
const SEARCH_BATCH = 50;

/** Recursive name search, bounded like the Rust command. */
function search(path: string, query: string, showHidden: boolean, limit: number, channel?: Channel<Entry[]>) {
  const node = lookup(path);
  if (!node || !node.dir) throw `Failed to read directory: No such file or directory (os error 2)`;
  const needle = fold(query.trim());
  const out: Entry[] = [];
  let stopped: "results" | "scanned" | null = null;
  let scanned = 0;
  // A queue, as in the backend: nearest folders first.
  const queue = needle ? [path.replace(/\/$/, "")] : [];
  walk: while (queue.length) {
    const dir = queue.shift()!;
    let entries: Entry[];
    try {
      entries = list(dir, showHidden);
    } catch {
      continue;
    }
    for (const entry of entries) {
      stopped = out.length >= limit ? "results" : scanned >= MAX_SEARCH_SCANNED ? "scanned" : null;
      if (stopped) break walk;
      scanned += 1;
      if (fold(entry.name).includes(needle)) out.push(entry);
      if (entry.is_dir && !entry.is_symlink) queue.push(entry.path);
    }
  }
  for (let i = 0; i < out.length; i += SEARCH_BATCH) channel?.onmessage(out.slice(i, i + SEARCH_BATCH));
  return { stopped, scanned };
}

type SearchSummaryMock = ReturnType<typeof search>;
/** A held search, answered when the test says so. */
interface HeldSearch {
  path: string;
  query: string;
  /** Walk now: send the batches, then end with the summary (or `summary` instead). */
  answer(summary?: Partial<SearchSummaryMock>): void;
  /** End with an error, as a vanished folder does. */
  fail(error: string): void;
}

/** Tests hold searches to answer them late (each waits in `held`), and count
 *  `cancel_search` calls. */
export const searchMock = {
  hold: false,
  held: [] as HeldSearch[],
  cancels: 0,
};

function searchCommand(a: Args): unknown {
  const run = () => search(localPath(str(a, "path")), str(a, "query"), a.showHidden as boolean, a.limit as number, a.onResults as Channel<Entry[]> | undefined);
  if (!searchMock.hold) return run();
  return new Promise((resolve, reject) => {
    searchMock.held.push({
      path: str(a, "path"),
      query: str(a, "query"),
      answer: (summary) => {
        try {
          resolve({ ...run(), ...summary });
        } catch (err) {
          reject(err);
        }
      },
      fail: reject,
    });
  });
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

/** Move `name` out of `parent` into the Trash and log where it came from
 *  (`origin`), as the backend logs everything it trashes; returns its path there. */
function trash(parent: Node & { dir: true }, name: string, origin: string): string {
  const bin = lookup(TRASH);
  if (!bin?.dir) throw "no trash";
  let target = name;
  for (let n = 2; bin.children[target]; n++) target = `${name} ${n}`;
  bin.children[target] = parent.children[name];
  delete parent.children[name];
  const trashed = `${TRASH}/${target}`;
  trashLog.set(trashed, { origin, time: Math.floor(Date.now() / 1000) });
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

// ---------------------------------------------------------------------------
// Operation progress (spec 2026-10-25-operation-progress)
// ---------------------------------------------------------------------------

type MockUpdate = { bytes: number; items: number; total: number | null; phase: "preparing" | "working" | "saving"; done: boolean };

/** Stand-in for Tauri's `Channel`: the backend calls `onmessage`. */
export class Channel<T> {
  onmessage: (message: T) => void = () => {};
}

type Held = { op: string; channel?: Channel<MockUpdate>; bytes: number; resolve: () => void; reject: (error: string) => void };
let holding = false;
let held: Held[] = [];
let measureFails = false;
let measureHolding = false;
/** Like the real app can: the last update arrives after the command returned. */
let lateDone = false;
/** Hold after doing the work: the item has landed but the command still runs. */
let holdingAfter = false;
const cancelledOps = new Set<string>();

function update(bytes: number, done: boolean): MockUpdate {
  return { bytes, items: done ? 1 : 0, total: null, phase: "working", done };
}

/** While tests hold, a long command waits here until released or cancelled. */
async function progressGate(a: Args): Promise<void> {
  const op = typeof a.op === "string" ? a.op : "";
  if (op && cancelledOps.has(op)) throw "Cancelled.";
  if (!holding || (holdingAfter && !a.__landed)) {
    if (holdingAfter) a.__landed = true;
    return;
  }
  const channel = a.onProgress as Channel<MockUpdate> | undefined;
  await new Promise<void>((resolve, reject) => held.push({ op, channel, bytes: 0, resolve, reject }));
}

/** The bytes a long command's final update carries, as the backend's does:
 *  every byte of a file it copies from the disk or downloads from Drive. */
function fileBytes(a: Args): number {
  if (typeof a.from !== "string") return 0;
  if (placeOf(a.from) === "drive") {
    const item = typeof a.to === "string" && placeOf(a.to) !== "drive" ? driveItemAt(a.from) : undefined;
    return item && !item.isDir && !item.doc ? item.size : 0;
  }
  const source = lookup(a.from);
  return source && !source.dir ? source.size : 0;
}

function gated(command: (a: Args) => unknown): (a: Args) => Promise<unknown> {
  return async (a) => {
    await progressGate(a);
    const done = update(fileBytes(a), true);
    const result = await command(a);
    if (holdingAfter) {
      holding = true;
      await progressGate(a);
    }
    const send = () => (a.onProgress as Channel<MockUpdate> | undefined)?.onmessage(done);
    if (lateDone) setTimeout(send, 0);
    else send();
    return result;
  };
}

function cancelOperation(op: string): void {
  cancelledOps.add(op);
  for (const h of held.filter((h) => h.op === op)) h.reject("Cancelled.");
  held = held.filter((h) => h.op !== op);
}

function measureNode(node: Node | undefined, total: { bytes: number; items: number }): void {
  if (!node) return;
  total.items += 1;
  if (node.dir) Object.values(node.children).forEach((child) => measureNode(child, total));
  else total.bytes += node.size;
}

/** Like the backend's `measure_paths`: Drive paths are asked of Drive, local
 *  paths are walked, and nothing else (shares, SSH servers, relative paths)
 *  counts. A Drive refusal fails the whole measure. */
function measure(paths: string[]): { bytes: number; items: number } {
  if (measureFails) throw "Could not measure.";
  const total = { bytes: 0, items: 0 };
  for (const path of paths) {
    if (placeOf(path) === "drive") measureDriveItem(driveItemToMeasure(path), total);
    else if (isLocal(path)) measureNode(lookup(path), total);
  }
  return total;
}

/** The Drive item at `path`, looked up by id as `cloud::transfer::measure` does. */
function driveItemToMeasure(path: string): DriveItem {
  const { ids, pending } = parseDrive(path);
  const id = ids[ids.length - 1];
  if (pending !== undefined || !id) throw "This Google Drive item has no id.";
  const item = driveItems.get(id);
  if (!item) throw "The Google Drive item no longer exists.";
  return item;
}

/** Every item counts; Google Docs files have no bytes; a folder's trashed
 *  children aren't listed. */
function measureDriveItem(item: DriveItem, total: { bytes: number; items: number }): void {
  total.items += 1;
  if (item.isDir) driveChildren(item.id).forEach((child) => measureDriveItem(child, total));
  else total.bytes += item.doc ? 0 : item.size;
}

// QR shares: preparing at the first status call, ready from the next one.
// Tests play the phone through `fileShareMock` (downloads, a running one).
interface MockFileShare {
  paths: string[];
  polls: number;
  downloads: number;
  sending: number | null;
  error?: string;
}
export const fileShareMock = {
  shares: new Map<string, MockFileShare>(),
  next: 1,
};

/** A small stand-in for a QR code: a finder pattern in each corner. */
const MOCK_QR =
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 29 29"><rect width="29" height="29" fill="#fff"/>` +
  [[2, 2], [20, 2], [2, 20]].map(([x, y]) =>
    `<rect x="${x}" y="${y}" width="7" height="7" fill="#000"/><rect x="${x + 1}" y="${y + 1}" width="5" height="5" fill="#fff"/><rect x="${x + 2}" y="${y + 2}" width="3" height="3" fill="#000"/>`).join("") +
  `<path d="M11 3h2v2h-2zM14 5h3v2h-3zM11 11h7v2h-7zM12 15h2v4h-2zM16 14h3v3h-3zM20 12h4v2h-4zM22 16h3v6h-3zM12 22h6v2h-6zM19 24h4v2h-4z" fill="#000"/></svg>`;

function prepareFileShare(paths: string[]): string {
  if (paths.length === 0) throw "Choose files to send.";
  if (!paths.every(isLocal)) throw "Only files on this computer can be sent with a QR code.";
  for (const path of paths) {
    const node = lookup(path);
    if (!node) throw "No such file or directory (os error 2)";
    if (node.dir) throw "Only files can be sent with a QR code.";
  }
  const id = `qr-${fileShareMock.next++}`;
  fileShareMock.shares.set(id, { paths, polls: 0, downloads: 0, sending: null });
  return id;
}

function fileShareStatus(id: string): unknown {
  const share = fileShareMock.shares.get(id);
  if (!share) return null;
  share.polls++;
  if (share.error) return { state: "failed", error: share.error };
  if (share.polls === 1) return { state: "preparing", progress: 0.5 };
  return {
    state: "ready",
    link: `https://example.invalid/get/#${id}`,
    qr: MOCK_QR,
    downloads: share.downloads,
    sending: share.sending,
  };
}

// Icons from the OS: the preview has none (the app draws its own, as for
// remote places), unless a test turns them on. Then folders and files share
// one icon per type and apps get their own, like Finder's; every request is
// logged. `delay` (ms) slows the id lookups, like a cold icon theme.
export const nativeIconMock = {
  enabled: false,
  delay: 0,
  log: { ids: [] as string[][], images: [] as string[] },
};

function mockIconId(path: string): string | null {
  if (!nativeIconMock.enabled || !isLocal(path)) return null;
  const node = lookup(path);
  if (!node) return null;
  if (node.dir) {
    if (/\.app$/i.test(path)) return `path:${path}`;
    // As Finder on macOS 26: a folder with visible items shows paper in it.
    return Object.keys(node.children).some((name) => !name.startsWith(".")) ? "full-folder" : "type:public.folder";
  }
  const ext = /\.([^./]+)$/.exec(path)?.[1]?.toLowerCase();
  return `type:${ext ?? "public.data"}`;
}

/** A tiny SVG that names its id, standing in for the OS's drawing. */
function mockIconImage(id: string, size: number): ArrayBuffer {
  nativeIconMock.log.images.push(`${id}@${size}`);
  if (!nativeIconMock.enabled) return new ArrayBuffer(0);
  const label = id.replace(/[<>&"]/g, "");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" data-id="${label}"><rect width="100%" height="100%" rx="8" fill="#888"/></svg>`;
  return new TextEncoder().encode(svg).buffer as ArrayBuffer;
}

// Thumbnails can be held open by tests, standing in for a slow Quick Look,
// and every request and cancellation is logged.
let thumbsHeld = false;
const heldThumbs = new Map<number, () => void>();
const thumbLog = { requested: [] as string[], cancelled: [] as number[] };

(globalThis as Record<string, unknown>).__mockThumbs = {
  log: thumbLog,
  hold(hold: boolean) {
    thumbsHeld = hold;
    if (hold) return;
    for (const resolve of heldThumbs.values()) resolve();
    heldThumbs.clear();
  },
  held: () => heldThumbs.size,
  reset() {
    this.hold(false);
    thumbLog.requested.length = 0;
    thumbLog.cancelled.length = 0;
  },
};

// Folder counts can be held by tests, standing in for a slow disk, and every
// request is logged (the folders asked for, one list per call).
let countsHeld = false;
const heldCounts: (() => void)[] = [];
const countLog = { asked: [] as string[][] };

(globalThis as Record<string, unknown>).__mockCounts = {
  log: countLog,
  hold(hold: boolean) {
    countsHeld = hold;
    if (!hold) heldCounts.splice(0).forEach((resolve) => resolve());
  },
  held: () => heldCounts.length,
  reset() {
    this.hold(false);
    countLog.asked.length = 0;
  },
};

async function itemCounts(paths: string[], showHidden: boolean): Promise<({ count: number; versioned: boolean } | null)[]> {
  countLog.asked.push([...paths]);
  if (countsHeld) await new Promise<void>((resolve) => heldCounts.push(resolve));
  return paths.map((path) => (isLocal(path) ? folderSummary(path, showHidden) : null));
}

(globalThis as Record<string, unknown>).__mockOps = {
  hold() {
    holding = true;
  },
  held: () => held.length,
  step(bytes: number) {
    for (const h of held) {
      h.bytes += bytes;
      h.channel?.onmessage(update(h.bytes, false));
    }
  },
  release() {
    const waiting = held;
    held = [];
    waiting.forEach((h) => h.resolve());
  },
  failMeasure(fail: boolean) {
    measureFails = fail;
  },
  holdMeasure() {
    measureHolding = true;
  },
  lateDone(late: boolean) {
    lateDone = late;
  },
  holdAfter() {
    holdingAfter = true;
  },
  reset() {
    holding = false;
    held = [];
    measureFails = false;
    measureHolding = false;
    lateDone = false;
    holdingAfter = false;
    cancelledOps.clear();
  },
};
const str = (args: Args, key: string) => args[key] as string;

function fileProperties(path: string) {
  const node = lookup(path);
  if (!node) throw "Could not read properties: No such file or directory (os error 2)";
  const i = path.lastIndexOf("/");
  const isDir = node.dir;
  const mode = mockMode(path);
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
    mode: mockModeString(mode, isDir, !!node.link),
    octal: mockOctal(mode),
    owner: mockAccessLabel(mode, 6, isDir),
    group: mockAccessLabel(mode, 3, isDir),
    others: mockAccessLabel(mode, 0, isDir),
  };
}

const mockModes = new Map<string, number>();

function mockMode(path: string): number {
  const known = mockModes.get(path);
  if (known !== undefined) return known;
  return lookup(path)?.dir ? 0o755 : 0o644;
}

function mockModeString(mode: number, isDir: boolean, isLink: boolean): string {
  if (isLink) return "lrwxrwxrwx";
  const triple = (shift: number) => {
    const bits = (mode >> shift) & 7;
    return `${bits & 4 ? "r" : "-"}${bits & 2 ? "w" : "-"}${bits & 1 ? "x" : "-"}`;
  };
  return `${isDir ? "d" : "-"}${triple(6)}${triple(3)}${triple(0)}`;
}

function mockOctal(mode: number): string {
  const special = mode & 0o7000;
  const low = mode & 0o777;
  return special ? (special | low).toString(8).padStart(4, "0") : low.toString(8).padStart(3, "0");
}

function mockAccessLabel(mode: number, shift: number, isDir: boolean): string {
  const bits = (mode >> shift) & 7;
  if (isDir) {
    if (bits === 7) return "Create and delete files";
    if (bits === 5) return "Access files";
    return bits === 0 ? "None" : "Access files";
  }
  if (bits === 6) return "Read and write";
  if (bits === 5) return "Read and execute";
  if (bits === 4) return "Read-only";
  if (bits === 2) return "Write-only";
  if (bits === 1) return "Execute";
  return bits === 0 ? "None" : "Custom";
}

function makeDir(path: string): void {
  const [parent, name] = parentAndName(path);
  if (!parent?.dir) throw "Failed to create directory: No such file or directory";
  if (parent.children[name]) throw `Path already exists: ${path}`;
  parent.children[name] = d({}, 0);
}

/**
 * The same refusals as the backend's copy/move (placement.rs
 * `check_transfer`), in the same order; a missing destination folder fails
 * after them, as the backend's copy does.
 */
function checkTransfer(from: string, to: string, overwrite: boolean): { node: Node; fromParent: Node & { dir: true }; toParent: Node & { dir: true }; toName: string } {
  const [fromParent, fromName] = parentAndName(from);
  const [toParent, toName] = parentAndName(to);
  const node = fromParent?.dir ? fromParent.children[fromName] : undefined;
  if (!node || !fromParent?.dir) throw `“${from}” no longer exists`;
  if (from === to) throw "The source and destination are the same";
  if (to.startsWith(from + "/")) throw "Cannot copy or move a folder into itself";
  // `.` and `..` always name an existing folder; the mock resolves no further.
  const dotted = toName === "." || toName === "..";
  const existing = dotted || Boolean(toParent?.dir && toParent.children[toName]);
  if (existing && !overwrite) throw `Target already exists: ${to}`;
  if (existing && !dotted && from.startsWith(to + "/")) throw "Cannot replace a folder with an item inside it";
  if (dotted || toName === "") throw `“${to}” is not a valid name for a local item.`;
  if (!toParent?.dir) throw "Failed to copy: No such file or directory";
  return { node, fromParent, toParent, toName };
}

function transfer(args: Args, move: boolean): void {
  const from = str(args, "from"), to = str(args, "to");
  const { node, fromParent, toParent, toName } = checkTransfer(from, to, Boolean(args.overwrite));
  if (toParent.children[toName]) trash(toParent, toName, to);
  toParent.children[toName] = move ? node : structuredClone(node);
  if (move) delete fromParent.children[from.slice(from.lastIndexOf("/") + 1)];
}

function trashEntry(path: string): string {
  const [parent, name] = parentAndName(path);
  if (!parent?.dir || !parent.children[name]) throw `“${path}” no longer exists`;
  return trash(parent, name, path);
}

/** Items per backend call, as `TRASH_CHUNK` in trash_ops.rs. */
const TRASH_CHUNK = 100;

/**
 * Like the backend: chunks of `TRASH_CHUNK`, progress counted per item, and
 * a cancel between chunks leaving the rest with "Cancelled.". Tests can hold
 * it before each chunk (__mockOps).
 */
async function trashEntries(a: Args): Promise<Array<{ trashed: string | null; error: string | null }>> {
  const paths = a.paths as string[];
  const channel = a.onProgress as Channel<MockUpdate> | undefined;
  const outcomes = paths.map(() => ({ trashed: null as string | null, error: "Cancelled." as string | null }));
  let items = 0;
  for (let start = 0; start < paths.length; start += TRASH_CHUNK) {
    try {
      await progressGate(a);
    } catch {
      break;
    }
    for (let i = start; i < Math.min(paths.length, start + TRASH_CHUNK); i++) {
      try {
        const trashed = placeOf(paths[i]) === "drive" ? driveTrash(paths[i], true) : trashEntry(localWrite(paths[i]));
        outcomes[i] = { trashed, error: null };
        items++;
      } catch (err) {
        outcomes[i] = { trashed: null, error: String(err) };
      }
    }
    channel?.onmessage({ bytes: 0, items, total: null, phase: "working", done: false });
  }
  channel?.onmessage({ bytes: 0, items, total: null, phase: "working", done: true });
  return outcomes;
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
  for (const logged of trashLog.keys()) if (isUnder(logged, path)) trashLog.delete(logged);
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

function renameMany(items: Array<{ from: string; to: string }>): void {
  const work: Array<{ from: string; to: string }> = [];
  const seenTo = new Set<string>();
  for (const item of items) {
    const [fromParent, fromName] = parentAndName(item.from);
    const [toParent, toName] = parentAndName(item.to);
    if (!fromParent?.dir || !fromParent.children[fromName]) throw `“${item.from}” no longer exists.`;
    if (fromParent !== toParent) throw "Items can only be renamed, not moved.";
    if (!toName) throw "A name can't be empty.";
    if (item.from === item.to) continue;
    if (seenTo.has(item.to)) throw "Two items would have the same name.";
    seenTo.add(item.to);
    if (toParent.children[toName]) throw `A file or folder named “${toName}” already exists.`;
    work.push(item);
  }
  const done: Array<{ from: string; to: string }> = [];
  for (const item of work) {
    try {
      renameEntry(item.from, item.to);
      done.push(item);
    } catch (err) {
      for (const back of done.reverse()) renameEntry(back.to, back.from);
      throw err;
    }
  }
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
  if (path.endsWith("/archive.zip")) return [v(2, 5_000_000, day, true), v(1, 4_000_000, 2 * day)];
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

/** Keep Versions works on this computer only: the path in `key`, after the
 *  backend's `place::local` refusals. */
const at = (a: Args, key = "path") => localPath(str(a, key));

const VERSION_COMMANDS: Record<string, (args: Args) => unknown> = {
  versioned_root: (a) => versionedRoot(str(a, "path")),
  make_versioned_dir: (a) => makeVersionedDir(localWrite(str(a, "path"))),
  list_versions: (a) => mockVersions(at(a)),
  version_count: (a) => mockVersions(at(a)).length,
  version_diff: (a) => versionDiff(at(a), a.version as number),
  restore_version_copy: (a) =>
    placeRestored(at(a), str(a, "name"), 1_700, `An item named “${str(a, "name")}” already exists`),
  open_version: (a) => {
    at(a);
    // A read-only copy the preview can read, as the backend makes it.
    ensureDir(OPENED).children[str(a, "name")] = f(1_700, 0);
    return `${OPENED}/${str(a, "name")}`;
  },
  replace_with_version: (a) => replaceWithVersion(at(a)),
  deleted_files: (a) => [{ path: `${at(a, "dir")}/draft.md`, deleted_at: now - 3 * 3600, size: 320 }],
  restore_deleted: (a) => {
    const path = at(a);
    return placeRestored(path, path.slice(path.lastIndexOf("/") + 1), 320, "Something is already at that path");
  },
  open_deleted: (a) => {
    at(a);
    return `${OPENED}/${str(a, "name")}`;
  },
  history_info: (a) => {
    at(a, "root");
    return { bytes: 12_400_000, versions: 214, files: 37 };
  },
  clear_history: (a) => clearHistory(at(a, "root")),
  stop_keeping_versions: (a) => stopKeepingVersions(at(a, "root")),
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
/** Connected accounts, in the order they were added, as the backend keeps them. */
let driveAccountList: MockDriveAccount[] = [];
/** The Google account the next sign-in in the browser picks (tests change it). */
let nextDriveEmail = "me@example.com";

/** A new account's key: "mock" (the paths the tests and the preview use) while free, then mock2, mock3… */
function freeDriveKey(): string {
  const used = new Set(driveAccountList.map((account) => driveKey(account.root_path)));
  let n = 1;
  while (used.has(n === 1 ? "mock" : `mock${n}`)) n++;
  return n === 1 ? "mock" : `mock${n}`;
}

/** Sign the next "Open Browser" in as `email`, as a second Google account would. */
export function setMockDriveEmail(email: string): void {
  nextDriveEmail = email;
}
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
  if (driveAccountList.length) throw "Disconnect Google Drive first.";
}
const driveFlows = new Set<string>();
let driveFlowSeq = 0;

// ----- Arbesk: one account, two collections; copies of local models go in (src-tauri/crates/arbesk/) -----

/** The wallet address every mock sign-in resolves to. */
export const ARBESK_MOCK_ADDRESS = `0x${"ab".repeat(20)}`;
const ARBESK_ORIGIN = "https://promptscad.com";

type MockArbeskAccount = { id: string; email: string; address: string; origin: string; chain_id: number; root_path: string };
/** `thumbnail`: saved with one, as the Studio does when it can render the model. */
type MockAsset = {
  id: string;
  name: string;
  ext: "gltf" | "3mf";
  huge?: boolean;
  thumbnail?: boolean;
  /** How many times it was saved (one when not given): save N of a model is N copies, 50 apart. */
  saves?: { prefix: string; count: number };
  /** The newest manifest's stored `version`, as any client may write it;
   *  not given, it is numbered as besk numbers saves (one per save). */
  version?: unknown;
  /** A scene's parts, each at the x offsets of its copies (tinted when a colour is given). */
  scene?: Array<{ key: string; format: "glb" | "3mf"; copies: Array<{ x: number; color?: string }> }>;
  /** Its other files, as Arbesk stores the entries (an invalid one
   *  included), in the newest save only. */
  representations?: MockRepresentation[];
  /** The save whose model the newest save holds, when a later save only
   *  changed its other files (an attach or a remove); the newest otherwise. */
  model?: number;
};

/** A representation entry as stored, loosely typed as another client may write it. */
type MockRepresentation = Record<string, unknown>;

/** The CID of an asset's model in its newest save: a file made from an
 *  earlier save names another one, so it reads out of date. */
const arbeskModelCid = (asset: MockAsset) => `bafy-${asset.id}-v${asset.model ?? asset.saves?.count ?? 1}`;

/** A representation entry of asset `asset` as `besk attach` records one.
 *  Its CID is letters and digits, as real ones are (a pinned path takes
 *  nothing else), and differs per asset and file: a CID means the bytes. */
function mockRep(asset: string, id: string, role: string, format: string, name: string, bytes: number, extra: MockRepresentation = {}): MockRepresentation {
  const encoding = ["gcode", "gltf", "obj", "step", "txt"].includes(format) ? "brotli" : "raw";
  const cid = `bafyrep${asset}x${id}`.replace(/[^A-Za-z0-9]/g, "");
  return { id, role, format, name, cid, bytes, encoding, created_at: (now - day) * 1000, ...extra };
}

/** The account's collections, as the backend lists them: Shapes, and one without a name. */
const ARBESK_COLLECTIONS: Array<{ token: string; name: string; assets: MockAsset[] }> = [
  {
    token: "1",
    name: "Shapes",
    assets: [
      {
        id: "box-g",
        name: "Box",
        ext: "gltf",
        thumbnail: true,
        representations: [mockRep("box-g", "stl", "mesh", "stl", "Box.stl", 684, { derived_from: { ref: "primary", cid: "bafy-box-g-v1" } })],
      },
      { id: "box-m", name: "Box", ext: "3mf" },
      // Saved by a client that writes a fractional version: no Version row.
      { id: "big", name: "Big", ext: "gltf", huge: true, version: 2.5 },
      // The spec's Bracket: a source, a fresh STL, a G-code for an Original
      // Prusa MK4 in PETG made from an earlier model (out of date), a 3MF,
      // and an entry File Browser can't read (left out, but stored).
      {
        id: "bracket",
        name: "Bracket",
        ext: "gltf",
        saves: { prefix: "bracket", count: 2 },
        representations: [
          mockRep("bracket", "source", "source", "blend", "Bracket.blend", 96_240),
          mockRep("bracket", "stl", "mesh", "stl", "Bracket.stl", 5_684, { derived_from: { ref: "primary", cid: "bafy-bracket-v2" } }),
          mockRep("bracket", "gcode-original-prusa-mk4-petg", "machine", "gcode", "Bracket_0.2mm_PETG_MK4.gcode", 1_204_118, {
            derived_from: { ref: "primary", cid: "bafy-bracket-v1" },
            tool: { name: "PrusaSlicer", version: "2.9.2" },
            profile: { printer: "Original Prusa MK4", material: "PETG", layer_mm: 0.2, nozzle_mm: 0.4 },
            stats: { print_time_s: 7380, filament_g: 38.5 },
          }),
          mockRep("bracket", "3mf", "mesh", "3mf", "Bracket.3mf", 2_204),
          { id: "Print Plan", role: "plan", format: "json" },
        ],
      },
    ],
  },
  {
    token: "2",
    name: "Untitled Collection",
    assets: [
      {
        id: "asset_1",
        name: "Cube",
        ext: "gltf",
        thumbnail: true,
        saves: { prefix: "cube", count: 3 },
        // The spec's Bracket set: a source, a fresh STL, a G-code made from
        // an earlier save (out of date), a 3MF, and an entry File Browser
        // can't read (left out, but stored).
        representations: [
          mockRep("asset_1", "source", "source", "blend", "Cube.blend", 48_120),
          mockRep("asset_1", "stl", "mesh", "stl", "Cube.stl", 3_412, { derived_from: { ref: "primary", cid: "bafy-asset_1-v3" } }),
          mockRep("asset_1", "gcode-original-prusa-mk4-petg", "machine", "gcode", "Cube_0.2mm_PETG_MK4.gcode", 812_331, {
            derived_from: { ref: "primary", cid: "bafy-asset_1-v2" },
            tool: { name: "PrusaSlicer", version: "2.9.2" },
            profile: { printer: "Original Prusa MK4", material: "PETG", layer_mm: 0.2, nozzle_mm: 0.4 },
            stats: { print_time_s: 7420, filament_g: 38.5 },
          }),
          mockRep("asset_1", "3mf", "mesh", "3mf", "Cube.3mf", 1_102),
          { id: "Print Plan", role: "plan", format: "json" },
        ],
      },
      // A scene: two copies of one model, the second 50 to the right (the model is 40 wide) and red, and a 3MF part.
      // Its manifest numbers it 5 though it has one save here: the Version
      // row shows the manifest's number, not a count of saves.
      {
        id: "table",
        name: "Table",
        ext: "gltf",
        version: 5,
        scene: [
          { key: "box-src", format: "glb", copies: [{ x: 0 }, { x: 50, color: "#ff0000" }] },
          { key: "box-3mf", format: "3mf", copies: [{ x: 0 }] },
        ],
      },
    ],
  },
];

let arbeskAccountList: MockArbeskAccount[] = [];
const arbeskFlows = new Map<string, string>();
let arbeskFlowSeq = 0;

/** `<id>~<name>`, both encoded as the backend writes them. */
const arbeskSegment = (id: string, name: string) => `${encodeCloudName(id)}~${encodeCloudName(name)}`;

/** The ids an `arbesk://` path names, after checking its account is
 *  connected: for an asset's other file (spec F4), its id and the CID it
 *  may be pinned to (`<id>@<cid>~<name>`). */
function arbeskIds(path: string): { address: string; token?: string; asset?: string; file?: string; pin?: string } {
  const [address, ...rest] = path.slice("arbesk://".length).replace(/\/+$/, "").split("/");
  if (!arbeskAccountList.some((account) => account.address === address?.toLowerCase())) {
    throw "Connect this Arbesk account first.";
  }
  const [token, asset, segment] = rest.map((part) => decodeURIComponent(part.split("~")[0]));
  if (segment === undefined) return { address, token, asset };
  // As the backend's `file_id`: a valid id, and a pin of letters and digits.
  const [, file, pin] = /^([a-z0-9-]{1,64})(?:@([A-Za-z0-9]+))?$/.exec(segment) ?? [];
  if (file === undefined || file === "primary") throw `“${path}” is not an Arbesk location.`;
  return { address, token, asset, file, pin };
}

function arbeskEntry(name: string, path: string, isDir: boolean, asset?: MockAsset): Entry {
  const entry: Entry = { name, path, is_dir: isDir, size: 0, modified: now - day, is_hidden: false, is_symlink: false };
  const summary = asset && arbeskSummary(asset);
  if (summary) entry.representations = summary;
  return entry;
}

// ----- An asset's other files (representations), as asset-core reads them -----

/** A valid stored entry, as `representationSchema` checks it (the parts this mock's entries exercise). */
type ValidRepresentation = {
  id: string; role: string; format: string; name: string; cid: string; bytes: number; encoding: string;
  derived_from?: { ref: string; cid: string }; tool?: object; profile?: object; stats?: object; created_at: number;
};

const REP_ID = /^[a-z0-9-]{1,64}$/;
const REP_ROLES = ["source", "mesh", "view", "machine", "other"];

function validRepresentation(entry: MockRepresentation): entry is ValidRepresentation & MockRepresentation {
  const text = (key: string) => typeof entry[key] === "string" && (entry[key] as string).length > 0;
  return REP_ID.test(String(entry.id)) && entry.id !== "primary" && REP_ROLES.includes(String(entry.role))
    && /^[a-z0-9]{1,16}$/.test(String(entry.format)) && text("name") && text("cid")
    && Number.isInteger(entry.bytes) && (entry.bytes as number) >= 0 && ["raw", "brotli"].includes(String(entry.encoding))
    && typeof entry.created_at === "number";
}

/** `listRepresentations`: the valid entries, in stored order. */
const arbeskFiles = (asset: MockAsset) => (asset.representations ?? []).filter(validRepresentation);

/** `isStale`: made from a model or an entry whose CID has changed (or is gone). */
function arbeskStale(asset: MockAsset, rep: ValidRepresentation): boolean {
  const from = rep.derived_from;
  if (!from) return false;
  const current = from.ref === "primary" ? arbeskModelCid(asset) : arbeskFiles(asset).find((r) => r.id === from.ref)?.cid;
  return current !== from.cid;
}

/** The listing's summary (spec F2); none without valid entries. */
function arbeskSummary(asset: MockAsset): Entry["representations"] {
  const reps = arbeskFiles(asset);
  if (!reps.length) return undefined;
  return {
    count: reps.length,
    stale: reps.filter((rep) => arbeskStale(asset, rep)).length,
    formats: [...new Set(reps.map((rep) => rep.format))],
  };
}

/** `arbesk_representations`: the asset's files in a save (its newest when
 *  `version` is null; earlier saves have none here). */
function arbeskRepresentations(path: string, version: string | null) {
  const asset = arbeskAsset(path);
  const saves = arbeskVersions(path).map((v) => v.id);
  if (version !== null && !saves.includes(version)) throw "This version is not in the asset.";
  if (version !== null && version !== saves[saves.length - 1]) return [];
  return arbeskFiles(asset).map((rep) => ({
    id: rep.id, role: rep.role, format: rep.format, name: rep.name, bytes: rep.bytes,
    stale: arbeskStale(asset, rep), derived_from: rep.derived_from?.ref ?? null,
    tool: rep.tool ?? null, profile: rep.profile ?? null, stats: rep.stats ?? null, created_at: rep.created_at,
    path: arbeskFilePath(path, rep.id, rep.cid, rep.name),
  }));
}

/** An asset's other file's path pinned to its CID, as the backend's
 *  `file_path` writes it: `<asset>/<id>@<cid>~<safe name>` (spec F4). */
function arbeskFilePath(asset: string, id: string, cid: string, name: string): string {
  return `${asset.replace(/\/+$/, "")}/${encodeCloudName(id)}@${encodeCloudName(cid)}~${encodeCloudName(safeFileName(name))}`;
}

/** Where the backend keeps opened copies of Arbesk files, in the fake filesystem. */
export const ARBESK_MOCK_FILES = "/tmp/opencode/mock-arbesk/files";

/** A file name as the backend makes one safe: no `/` or NUL, never empty, `.` or `..`. */
function safeFileName(name: string): string {
  const safe = name.replace(/[/\0]/g, "_");
  return safe === "" || safe === "." || safe === ".." ? "_" : safe;
}

/** Opening an asset's other file (spec F5): fetched into the app cache,
 *  `…/arbesk/files/<cid>/<name>`, where `open_with` and `preview_bytes`
 *  find it; a pinned path whose entry moved is refused. */
function arbeskFileOpen(path: string): string {
  const { file, pin } = arbeskIds(path);
  const rep = arbeskFiles(arbeskAsset(path)).find((r) => r.id === file);
  if (pin !== undefined && rep?.cid !== pin) {
    const shown = decodeURIComponent(path.slice(path.lastIndexOf("/") + 1).split("~").slice(1).join("~"));
    throw `“${shown}” changed in Arbesk since it was copied.`;
  }
  if (!rep) throw "This file is no longer in the asset.";
  const dir = ensureDir(`${ARBESK_MOCK_FILES}/${rep.cid}`);
  const name = safeFileName(rep.name);
  dir.children[name] ??= f(rep.bytes, 0);
  return `${ARBESK_MOCK_FILES}/${rep.cid}/${name}`;
}

function listArbesk(path: string): Entry[] {
  const { address, token, asset } = arbeskIds(path);
  if (asset) throw `“${path}” is a file, not a folder.`;
  const root = `arbesk://${address}`;
  if (!token) return ARBESK_COLLECTIONS.map((c) => arbeskEntry(c.name, `${root}/${arbeskSegment(c.token, c.name)}`, true));
  const collection = ARBESK_COLLECTIONS.find((c) => c.token === token);
  if (!collection) throw "The chain refused this collection.";
  const base = path.replace(/\/+$/, "");
  return collection.assets.map((a) => arbeskEntry(`${a.name}.${a.ext}`, `${base}/${arbeskSegment(a.id, `${a.name}.${a.ext}`)}`, false, a));
}

/** `version_of`: the newest manifest's `version` when it is a JSON number,
 *  else 1 (an unset mock version is the save count, as besk numbers them). */
function arbeskVersionOf(asset: MockAsset): number {
  if (asset.version === undefined) return asset.saves?.count ?? 1;
  return typeof asset.version === "number" ? asset.version : 1;
}

/** `whole_version`: the version shown, when whole, 0 or more and within a u32. */
function arbeskWholeVersion(asset: MockAsset): number | undefined {
  const version = arbeskVersionOf(asset);
  return Number.isInteger(version) && version >= 0 && version <= 0xffff_ffff ? version : undefined;
}

/** `file_properties` for an Arbesk asset (spec F7): its name, collection,
 *  save date and version (`whole_version`'s rule, left out when it has
 *  none); anything else is refused. */
function arbeskProperties(path: string) {
  if (!isArbeskAsset(path)) {
    arbeskIds(path);
    throw "Properties aren't available for Arbesk yet.";
  }
  const asset = arbeskAsset(path);
  const saves = arbeskVersions(path);
  const version = arbeskWholeVersion(asset);
  return {
    name: `${asset.name}.${asset.ext}`, path, parent: path.slice(0, path.lastIndexOf("/")),
    is_dir: false, is_symlink: false, symlink_target: null, size: 0, item_count: null,
    modified: Math.floor(saves[saves.length - 1].timestamp / 1000), accessed: 0, created: 0,
    mode: "", octal: "", owner: "", group: "", others: "",
    ...(version === undefined ? {} : { version }),
  };
}

function arbeskAsset(path: string): MockAsset {
  const { token, asset } = arbeskIds(path);
  const found = ARBESK_COLLECTIONS.find((c) => c.token === token)?.assets.find((a) => a.id === asset);
  if (!found) throw "This asset is no longer in the collection.";
  return found;
}

/** An asset's built file: a self-contained glTF (as Arbesk's are) or a 3MF. */
function arbeskBytes(path: string, limit: number): Uint8Array {
  const asset = arbeskAsset(path);
  if (asset.huge) throw "This file is too large to preview.";
  if (asset.ext === "3mf") return sample3mf();
  // As the backend builds an Arbesk glTF for the preview: one binary GLB.
  const bytes = sampleGlb();
  if (bytes.length > limit) throw "This file is too large to preview.";
  return bytes;
}

/** An asset's built file as a copy places it (its export's size). */
function arbeskModelNode(path: string): Node {
  const asset = arbeskAsset(path);
  if (asset.huge) throw "This asset is too large to open.";
  return f(asset.ext === "3mf" ? sample3mf().length : sampleGlb().length, 0);
}

/** Where the mock keeps an asset's built file, as the backend's `export`. */
export const ARBESK_MOCK_BUILDS = "/tmp/opencode/mock-arbesk/builds";

/** `arbesk_export` (spec F8): what Open With hands an application, in the
 *  app cache: an asset's built file, or one of its other files fetched
 *  (a pinned path whose entry moved refused, as `open_target` does). */
function arbeskExport(path: string): string {
  if (arbeskIds(path).file !== undefined) return arbeskFileOpen(path);
  const asset = arbeskAsset(path);
  const node = arbeskModelNode(path);
  const name = `${asset.name}.${asset.ext}`;
  ensureDir(`${ARBESK_MOCK_BUILDS}/${asset.id}`).children[name] ??= node;
  return `${ARBESK_MOCK_BUILDS}/${asset.id}/${name}`;
}

/**
 * copy_entry from Arbesk (place::copy): an asset or one of its other files
 * (spec F9: pinned paths whose entry moved refused), only to a folder on
 * this computer or in Drive. Its file is built (Client::export), then placed
 * as any copy is: a name taken asks first, and Replace trashes the old item.
 */
function arbeskCopy(a: Args): string {
  const from = str(a, "from"), to = str(a, "to");
  const overwrite = Boolean(a.overwrite);
  writable(to);
  const ids = arbeskIds(from);
  if (!ids.asset) throw "Copying Arbesk collections comes next.";
  const node = ids.file === undefined ? arbeskModelNode(from) : f(lookup(arbeskFileOpen(from))!.size, 0);
  if (placeOf(to) === "drive") return uploadNode(node, to, overwrite);
  const [toParent, toName] = parentAndName(localPath(to));
  if (!toParent?.dir) throw "The destination folder no longer exists.";
  if (toParent.children[toName]) {
    if (!overwrite) throw `Target already exists: ${to}`;
    trash(toParent, toName, to);
  }
  toParent.children[toName] = node;
  return to;
}

const ARBESK_ONLY_MODELS = "Only .gltf, .glb and .3mf files on this computer can be copied to Arbesk.";
const ARBESK_PASTE_INTO = "Paste into a collection.";
const ARBESK_SIGN_IN = "Sign in to Arbesk again to copy files into it.";
const ARBESK_CREATE_SIGN_IN = "Sign in to Arbesk again to create a collection.";
const ARBESK_SHARE_SIGN_IN = "Sign in to Arbesk again to share a collection.";
const ARBESK_TOO_LARGE = "This file is too large for Arbesk (100 MB at most).";
const ARBESK_MAX_UPLOAD = 100 * 1024 * 1024;

/** The session copies into Arbesk need (D4): tests take it away, or let the next write find it expired. */
const arbeskSession = { signedIn: true, expireNext: false };
(globalThis as Record<string, unknown>).__mockArbesk = {
  get signedIn() {
    return arbeskSession.signedIn;
  },
  set signedIn(value: boolean) {
    arbeskSession.signedIn = value;
  },
  /** The next write is refused as Arbesk refuses an expired session (401), which forgets it. */
  expireOnce() {
    arbeskSession.expireNext = true;
  },
  /** For tests of a virtualized view: a collection "Many" (token 9) of
   *  `count` assets, every third without other files, one with a fresh
   *  STL, one with an out-of-date one. Its path. */
  addManyAssets(count: number): string {
    const assets: MockAsset[] = Array.from({ length: count }, (_, i) => {
      const id = `m${i}`;
      const made = i % 3 === 2 ? "bafy-old" : `bafy-${id}-v1`;
      const stl = mockRep(id, "stl", "mesh", "stl", `Part ${i}.stl`, 100, { derived_from: { ref: "primary", cid: made } });
      return { id, name: `Part ${String(i).padStart(4, "0")}`, ext: "gltf", representations: i % 3 === 0 ? undefined : [stl] };
    });
    ARBESK_COLLECTIONS.push({ token: "9", name: "Many", assets });
    return `arbesk://${ARBESK_MOCK_ADDRESS}/${arbeskSegment("9", "Many")}`;
  },
  /** For copy-out and attach tests: a glTF asset `id` named `name` in
   *  collection `token`, with `files` (each a mesh, made from nothing) and
   *  `unreadable` entries stored after them as given. */
  addAsset(token: string, id: string, name: string, files: Array<{ id: string; format: string; name: string; bytes: number }>, unreadable: MockRepresentation[] = []): void {
    const representations = [...files.map((file) => mockRep(id, file.id, "mesh", file.format, file.name, file.bytes)), ...unreadable];
    ARBESK_COLLECTIONS.find((c) => c.token === token)!.assets.push({ id, name, ext: "gltf", representations });
  },
  /** The next attach or remove meets another save twice: CONFLICT, nothing changes. */
  conflictNext(): void {
    arbeskWrites.conflictNext = true;
  },
  /** The next attach or remove fails uploading: nothing changes. */
  failUploadNext(): void {
    arbeskWrites.failUploadNext = true;
  },
  /** As a save elsewhere would: the file `file` of asset `asset` now holds other bytes (another CID). */
  changeFile(token: string, asset: string, file: string): void {
    const found = ARBESK_COLLECTIONS.find((c) => c.token === token)?.assets.find((a) => a.id === asset);
    const rep = found?.representations?.find((r) => r.id === file);
    if (rep) rep.cid = `${String(rep.cid)}x2`;
  },
};
let arbeskAssetSeq = 0;
let arbeskCollectionSeq = 2;
/** Each collection's collaborators, as sharing adds or changes them. */
const arbeskCollaborators = new Map<string, Array<{ email: string; role: number }>>();

/** The collection and listed name of a copy's target, `<collection>/~<name>`
 *  (location::parse_target). */
function arbeskTarget(to: string): { collection: (typeof ARBESK_COLLECTIONS)[number]; dir: string; name: string } {
  const trimmed = to.replace(/\/+$/, "");
  const dir = trimmed.slice(0, trimmed.lastIndexOf("/"));
  const last = trimmed.slice(dir.length + 1);
  let name = "";
  try {
    name = last.startsWith("~") ? decodeURIComponent(last.slice(1)) : "";
  } catch {
    /* a malformed escape names nothing */
  }
  const { token, asset } = arbeskIds(dir);
  if (!name || !token || asset) throw ARBESK_PASTE_INTO;
  const collection = ARBESK_COLLECTIONS.find((c) => c.token === token);
  if (!collection) throw "The chain refused this collection.";
  return { collection, dir, name };
}

/** The session a write needs, as Client::session_for answers; an
 *  expired one is refused once and forgotten, as a 401 does. */
function arbeskSessionFor(needed = ARBESK_SIGN_IN): void {
  if (arbeskSession.expireNext) {
    arbeskSession.expireNext = false;
    arbeskSession.signedIn = false;
  }
  if (!arbeskSession.signedIn) throw needed;
}

/** A new asset's id, as the Studio makes one: `asset_<n>`, not yet taken. */
function freshArbeskId(): string {
  const taken = new Set(ARBESK_COLLECTIONS.flatMap((c) => c.assets.map((a) => a.id)));
  let id: string;
  do id = `asset_${++arbeskAssetSeq}`;
  while (taken.has(id));
  return id;
}

/**
 * copy_entry into Arbesk (place::copy's IntoArbesk, Client::add): a
 * local .gltf, .glb or .3mf becomes an asset named after the target, a .glb
 * stored as glTF. Refused in the backend's order: the source, the target,
 * the account, the session, the file, a taken name. Replace keeps the asset
 * and saves the file as its next version (D3), with its other files (spec
 * F15), which then read out of date.
 */
function arbeskAdd(a: Args): string {
  const from = str(a, "from");
  if (placeOf(from) !== "local") throw ARBESK_ONLY_MODELS;
  localPath(from);
  const { collection, dir, name } = arbeskTarget(str(a, "to"));
  arbeskSessionFor();
  const source = lookup(from);
  if (!source || source.dir || !/\.(gltf|glb|3mf)$/i.test(from)) throw ARBESK_ONLY_MODELS;
  if (source.size > ARBESK_MAX_UPLOAD) throw ARBESK_TOO_LARGE;
  const ext: MockAsset["ext"] = /\.3mf$/i.test(from) ? "3mf" : "gltf";
  const taken = collection.assets.find((asset) => `${asset.name}.${asset.ext}` === name);
  if (taken && !a.overwrite) throw `Target already exists: ${name}`;
  (a.onProgress as Channel<MockUpdate> | undefined)?.onmessage({ bytes: source.size, items: 0, total: null, phase: "saving", done: false });
  if (taken) {
    const saves = taken.saves ?? { prefix: taken.id, count: 1 };
    taken.saves = { ...saves, count: saves.count + 1 };
    // A new model: the newest save's own.
    delete taken.model;
    // `next_version`: one more than the stored version's number.
    if (taken.version !== undefined) taken.version = arbeskVersionOf(taken) + 1;
    return `${dir}/${arbeskSegment(taken.id, name)}`;
  }
  const dot = name.lastIndexOf(".");
  const asset: MockAsset = { id: freshArbeskId(), name: (dot > 0 ? name.slice(0, dot) : name) || "Uploaded Asset", ext };
  collection.assets.push(asset);
  return `${dir}/${arbeskSegment(asset.id, `${asset.name}.${asset.ext}`)}`;
}

// ----- Adding files to an asset and removing them (spec F10–F12) -----

const ARBESK_ATTACH_SIGN_IN = "Sign in to Arbesk again to add files to it.";
const ARBESK_DETACH_SIGN_IN = "Sign in to Arbesk again to remove files from it.";
const ARBESK_ONLY_LOCAL = "Only files on this computer can be added to an asset.";
const ARBESK_MAX_FILE = 256 * 1024 * 1024;

/** `default_role`: what a file of a format usually is. */
function mockDefaultRole(format: string): string {
  if (["blend", "step", "stp", "f3d", "fcstd", "scad", "iges", "igs"].includes(format)) return "source";
  if (["stl", "obj", "ply", "3mf"].includes(format)) return "mesh";
  if (["glb", "gltf", "usdz"].includes(format)) return "view";
  if (["gcode", "bgcode"].includes(format)) return "machine";
  return "other";
}

/** A local file a paste would add, as `attach::local_file` reads it (the
 *  mock reads no G-code header: the id is the role's or the format's). */
function mockLocalFile(path: string) {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const node = placeOf(path) === "local" ? lookup(path) : undefined;
  if (!node || node.dir || !path.startsWith("/")) throw ARBESK_ONLY_LOCAL;
  const dot = name.lastIndexOf(".");
  const format = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  if (!/^[a-z0-9]{1,16}$/.test(format)) throw `“${name}” has no file type to record.`;
  if (node.size > ARBESK_MAX_FILE) throw `“${name}” is larger than 256 MB.`;
  const role = mockDefaultRole(format);
  return { path, name, format, role, bytes: node.size, id: role === "source" ? "source" : format };
}

/** An asset's stored ids: valid ones with their CID, and those of entries
 *  File Browser can't read. */
function storedIds(asset: MockAsset) {
  const valid = new Map<string, string>();
  for (const rep of arbeskFiles(asset)) if (!valid.has(rep.id)) valid.set(rep.id, rep.cid);
  const all = (asset.representations ?? []).flatMap((rep) => (typeof rep.id === "string" ? [rep.id] : []));
  return { valid, all: new Set(all), unreadable: new Set(all.filter((id) => !valid.has(id))) };
}

const heldByUnreadable = (id: string) => `This asset already has a file “${id}” that File Browser can't show.`;

/** `arbesk_attach_plan`: each file as it would be recorded, and its clash. */
function arbeskAttachPlan(path: string, files: string[]) {
  const asset = arbeskAsset(path);
  const local = files.map(mockLocalFile);
  const stored = storedIds(asset);
  const infos = arbeskRepresentations(path, null);
  return local.map((file) => {
    let clash: unknown = null;
    if (stored.unreadable.has(file.id)) clash = { kind: "unreadable", message: heldByUnreadable(file.id) };
    else if (stored.valid.has(file.id)) clash = { kind: "valid", cid: stored.valid.get(file.id), existing: infos.find((info) => info.id === file.id) };
    return { ...file, clash };
  });
}

/** The first `<id>-2`, `<id>-3`… neither stored nor taken by the paste. */
function mockFreeId(id: string, taken: Set<string>): string {
  for (let n = 2; ; n++) {
    const suffix = `-${n}`;
    const candidate = `${id.slice(0, 64 - suffix.length).replace(/-+$/, "")}${suffix}`;
    if (!taken.has(candidate)) return candidate;
  }
}

/** Test switches: the next attach or remove meets another save (CONFLICT,
 *  twice: nothing changes) or a failed upload; both change nothing. */
const arbeskWrites = { conflictNext: false, failUploadNext: false };

/** A new mock version of `asset` that keeps its model: one more save. */
function newFilesVersion(asset: MockAsset): void {
  const saves = asset.saves ?? { prefix: asset.id, count: 1 };
  asset.model ??= saves.count;
  asset.saves = { ...saves, count: saves.count + 1 };
  if (asset.version !== undefined) asset.version = arbeskVersionOf(asset) + 1;
}

/** What a write leaves (`attach::Attached`). */
function attached(path: string, refused: string[]) {
  const asset = arbeskAsset(path);
  return { files: arbeskRepresentations(path, null), representations: arbeskSummary(asset) ?? null, refused };
}

/** Once a write's checks pass: the test switches. A failed upload names
 *  the file going up: an attach's first file, a remove's asset (its
 *  manifests are all it uploads). */
function arbeskWriteChecks(name: string, removing: boolean, uploading = name): void {
  if (arbeskWrites.conflictNext) {
    arbeskWrites.conflictNext = false;
    throw removing
      ? `“${name}” changed while the file was being removed. Nothing was changed; try again.`
      : `“${name}” changed while the files were being added. Nothing was changed; try again.`;
  }
  if (arbeskWrites.failUploadNext) {
    arbeskWrites.failUploadNext = false;
    throw `Couldn't upload “${uploading}” to IPFS: The server answered 500.`;
  }
}

/** `arbesk_attach` (spec F10, F11): the files as one new version, with
 *  the backend's rules: the session, each file, the ids settled against
 *  the stored ones (Replace only while the entry holds the file agreed to,
 *  Keep Both otherwise and when nobody answered), room for 32. */
function arbeskAttach(a: Args) {
  const path = str(a, "path");
  const asset = arbeskAsset(path);
  const name = `${asset.name}.${asset.ext}`;
  arbeskSessionFor(ARBESK_ATTACH_SIGN_IN);
  const adding = a.files as Array<{ path: string; choice?: { kind: string; cid?: string } }>;
  const local = adding.map((file) => mockLocalFile(file.path));
  arbeskWriteChecks(name, false, local[0]?.name);
  const stored = storedIds(asset);
  const taken = new Set(stored.all);
  const given = new Set<string>();
  const refused: string[] = [];
  const settled: Array<{ file: ReturnType<typeof mockLocalFile>; id: string }> = [];
  for (const [i, file] of local.entries()) {
    const choice = adding[i].choice;
    if (stored.unreadable.has(file.id)) {
      refused.push(heldByUnreadable(file.id));
      continue;
    }
    let id = file.id;
    if (stored.valid.has(id)) {
      if (choice?.kind === "skip") continue;
      const replace = choice?.kind === "replace" && choice.cid === stored.valid.get(id) && !given.has(id);
      if (!replace) id = mockFreeId(id, new Set([...taken, ...given]));
    } else if (given.has(id)) id = mockFreeId(id, new Set([...taken, ...given]));
    given.add(id);
    settled.push({ file, id });
  }
  if (settled.length === 0) throw `Nothing was added to “${name}”.`;
  const adds = settled.filter(({ id }) => !stored.all.has(id)).length;
  if ((asset.representations?.length ?? 0) + adds > 32) throw "An asset holds at most 32 files.";
  const list = [...(asset.representations ?? [])];
  for (const { file, id } of settled) {
    const derived = ["machine", "mesh"].includes(file.role) ? { derived_from: { ref: "primary", cid: arbeskModelCid(asset) } } : {};
    const entry = mockRep(asset.id, id, file.role, file.format, file.name, file.bytes, derived);
    entry.cid = `${String(entry.cid)}v${(asset.saves?.count ?? 1) + 1}`;
    const at = list.findIndex((rep) => rep.id === id);
    const kept = list.filter((rep) => rep.id !== id);
    kept.splice(at >= 0 ? at : kept.length, 0, entry);
    list.splice(0, list.length, ...kept);
  }
  asset.representations = list;
  newFilesVersion(asset);
  return attached(path, refused);
}

/** `arbesk_detach` (spec F12): the next version without the entry, only
 *  while it still holds the file `cid` the row showed; the list dropped
 *  once it empties. */
function arbeskDetach(a: Args) {
  const path = str(a, "path");
  const asset = arbeskAsset(path);
  arbeskSessionFor(ARBESK_DETACH_SIGN_IN);
  arbeskWriteChecks(`${asset.name}.${asset.ext}`, true);
  const id = str(a, "id");
  const stored = storedIds(asset);
  if (!stored.all.has(id)) throw "This file is no longer in the asset.";
  if (stored.valid.get(id) !== str(a, "cid")) throw "This file was replaced in the asset since it was shown. Nothing was removed.";
  const kept = (asset.representations ?? []).filter((rep) => rep.id !== id);
  asset.representations = kept.length ? kept : undefined;
  newFilesVersion(asset);
  return attached(path, []);
}

/** Whether a pending name is listed in its collection (Client::exists). */
function arbeskPendingExists(path: string): boolean {
  try {
    const { collection, name } = arbeskTarget(path);
    return collection.assets.some((asset) => `${asset.name}.${asset.ext}` === name);
  } catch {
    return false;
  }
}

/** The thumbnail an asset was saved with, as PNG: a shaded cube on the
 *  Studio's backdrop, 16:9 and at most 512 x 288 like the Studio's (the
 *  backend never enlarges one). Empty when it has none or can't be read,
 *  as the backend answers (never an error). */
async function arbeskThumbnail(path: string, size: number): Promise<ArrayBuffer> {
  let asset: MockAsset;
  try {
    asset = arbeskAsset(path);
  } catch {
    return new ArrayBuffer(0);
  }
  if (!asset.thumbnail) return new ArrayBuffer(0);
  const w = Math.min(size, 512);
  const h = Math.round((w * 9) / 16);
  const canvas = new OffscreenCanvas(w, h);
  const ctx = canvas.getContext("2d")!;
  ctx.fillStyle = "#2a2b2f";
  ctx.fillRect(0, 0, w, h);
  const r = h * 0.35;
  const face = (color: string, points: Array<[number, number]>) => {
    ctx.fillStyle = color;
    ctx.beginPath();
    points.forEach(([x, y], i) => (i ? ctx.lineTo(w / 2 + x * r, h / 2 + y * r) : ctx.moveTo(w / 2 + x * r, h / 2 + y * r)));
    ctx.closePath();
    ctx.fill();
  };
  face("#f0a64b", [[0, -1], [0.87, -0.5], [0, 0], [-0.87, -0.5]]);
  face("#c27f2f", [[-0.87, -0.5], [0, 0], [0, 1], [-0.87, 0.5]]);
  face("#9a6322", [[0.87, -0.5], [0.87, 0.5], [0, 1], [0, 0]]);
  return (await canvas.convertToBlob({ type: "image/png" })).arrayBuffer();
}

/** An asset's saves, oldest first, as `arbesk_versions` answers: a day apart, the last one now. */
function arbeskVersions(path: string): Array<{ id: string; timestamp: number }> {
  const asset = arbeskAsset(path);
  const { prefix, count } = asset.saves ?? { prefix: asset.id, count: 1 };
  return Array.from({ length: count }, (_, i) => ({ id: `${prefix}-v${i + 1}`, timestamp: (now - (count - 1 - i) * day) * 1000 }));
}

/** An asset's scene as the backend answers it, for one save (the newest
 *  when `version` is null): a single model is one part, one copy per save. */
function arbeskScene(path: string, version: string | null = null) {
  const asset = arbeskAsset(path);
  const saves = arbeskVersions(path).map((v) => v.id);
  const save = version === null ? saves.length : saves.indexOf(version) + 1;
  if (save === 0) throw "This version is not in the asset.";
  const copies = Array.from({ length: save }, (_, i) => ({ x: i * 50 }));
  const parts = asset.scene ?? [{ key: `${asset.id}-src`, format: asset.ext === "3mf" ? "3mf" : "glb", copies }];
  return {
    parts: parts.map((part) => ({
      key: part.key,
      format: part.format,
      placements: part.copies.map((copy) => ({
        matrix: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, copy.x, 0, 0, 1],
        color: copy.color ?? null,
        mesh_colors: {},
        scale: [1, 1, 1],
      })),
    })),
  };
}

/** One part's bytes, as the preview loads them: a GLB or a 3MF. */
function arbeskPart(path: string, version: string | null, key: string, limit: number): ArrayBuffer {
  const part = arbeskScene(path, version).parts.find((p) => p.key === key);
  if (!part) throw "This part is not in the asset.";
  if (arbeskAsset(path).huge) throw "This file is too large to preview.";
  const bytes = part.format === "3mf" ? sample3mf() : sampleGlb();
  if (bytes.length > limit) throw "This file is too large to preview.";
  return bytes.slice().buffer;
}

const ARBESK_COMMANDS: Record<string, (args: Args) => unknown> = {
  arbesk_versions: (a) => arbeskVersions(str(a, "path")),
  arbesk_scene: (a) => arbeskScene(str(a, "path"), (a.version as string | null | undefined) ?? null),
  arbesk_representations: (a) => arbeskRepresentations(str(a, "path"), (a.version as string | null | undefined) ?? null),
  arbesk_export: gated((a) => arbeskExport(str(a, "path"))),
  arbesk_attach_plan: (a) => arbeskAttachPlan(str(a, "path"), a.files as string[]),
  arbesk_attach: gated(arbeskAttach),
  arbesk_detach: gated(arbeskDetach),
  arbesk_part: (a) => arbeskPart(str(a, "path"), (a.version as string | null | undefined) ?? null, str(a, "key"), Number(a.limit)),
  arbesk_accounts: () => arbeskAccountList.map((account) => ({ ...account })),
  arbesk_connect_start: (a) => {
    const email = str(a, "email").trim().toLowerCase();
    if (!email.includes("@") || /\s/.test(email)) throw "Enter the email address you use with Arbesk.";
    const flow_id = `arbesk-flow-${++arbeskFlowSeq}`;
    arbeskFlows.set(flow_id, email);
    return { flow_id, sign_in_url: `${ARBESK_ORIGIN}/api/v1/cli-auth?email=${encodeURIComponent(email)}&port=4242` };
  },
  arbesk_connect_finish: (a) => {
    const email = arbeskFlows.get(str(a, "flowId"));
    if (!email) throw "This sign-in attempt is no longer active.";
    arbeskFlows.delete(str(a, "flowId"));
    const known = arbeskAccountList.find((account) => account.address === ARBESK_MOCK_ADDRESS);
    const account: MockArbeskAccount = {
      id: known?.id ?? "arbesk-account",
      email,
      address: ARBESK_MOCK_ADDRESS,
      origin: ARBESK_ORIGIN,
      chain_id: 84532,
      root_path: `arbesk://${ARBESK_MOCK_ADDRESS}`,
    };
    arbeskAccountList = [...arbeskAccountList.filter((item) => item !== known), account];
    // Signing in keeps a new session (D4).
    arbeskSession.signedIn = true;
    return { ...account };
  },
  arbesk_create_collection: (a) => {
    const { address, token, asset } = arbeskIds(str(a, "path"));
    if (token || asset) throw "Create collections in the account's folder.";
    arbeskSessionFor(ARBESK_CREATE_SIGN_IN);
    const name = str(a, "name").trim();
    if (!name) throw "Enter a name for the collection.";
    if (ARBESK_COLLECTIONS.some((collection) => collection.name === name)) throw `A collection named “${name}” already exists.`;
    const created = String(++arbeskCollectionSeq);
    ARBESK_COLLECTIONS.push({ token: created, name, assets: [] });
    return `arbesk://${address}/${arbeskSegment(created, name)}`;
  },
  arbesk_share_collection: (a) => {
    const { token, asset } = arbeskIds(str(a, "path"));
    if (!token || asset) throw "Share a collection.";
    if (!ARBESK_COLLECTIONS.some((collection) => collection.token === token)) throw "The chain refused this collection.";
    arbeskSessionFor(ARBESK_SHARE_SIGN_IN);
    const email = str(a, "email").trim().toLowerCase();
    if (!email.includes("@")) throw "Enter an email address.";
    if (email === "unknown@example.com") throw "No Arbesk account found for that email. They need to sign in to Arbesk at least once.";
    const role = Number(a.role);
    if (role !== 1 && role !== 2) throw "Choose Viewer or Editor.";
    const list = arbeskCollaborators.get(token) ?? [];
    const found = list.find((member) => member.email === email);
    if (found?.role === role) throw `That account is already a${role === 2 ? "n editor" : " viewer"}.`;
    const added = !found;
    if (found) found.role = role;
    else {
      list.push({ email, role });
      arbeskCollaborators.set(token, list);
    }
    const address = `0x${[...email].map((c) => c.charCodeAt(0).toString(16).padStart(2, "0")).join("").padEnd(40, "0").slice(0, 40)}`;
    return { address, role, added };
  },
  arbesk_signed_in: (a) => {
    try {
      arbeskIds(str(a, "path"));
    } catch {
      return false;
    }
    return arbeskSession.signedIn;
  },
  arbesk_connect_cancel: (a) => void arbeskFlows.delete(str(a, "flowId")),
  arbesk_disconnect: (a) => {
    arbeskAccountList = arbeskAccountList.filter((account) => account.id !== str(a, "account"));
  },
};

const DRIVE_COMMANDS: Record<string, (args: Args) => unknown> = {
  drive_accounts: () => driveAccountList.map((account) => ({ ...account })),
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
    // Signing in again for the same account keeps its id and paths, and so
    // does a fresh sign-in to a Google account that is already connected.
    const reconnect = typeof a.account === "string" ? a.account : null;
    const email = nextDriveEmail;
    const known =
      driveAccountList.find((account) => account.id === reconnect) ??
      (reconnect ? undefined : driveAccountList.find((account) => account.email.toLowerCase() === email.toLowerCase()));
    const key = known ? driveKey(known.root_path) : freeDriveKey();
    const account: MockDriveAccount = {
      id: known?.id ?? (key === "mock" ? "mock-account" : `${key}-account`),
      email,
      name: email.split("@")[0],
      root_id: "mock-root",
      root_name: "My Drive",
      root_path: DRIVE_ROOT.replace("gdrive://mock/", `gdrive://${key}/`),
      scopes: [DRIVE_SCOPE],
      connected_at: now,
    };
    if (known) driveAccountList = driveAccountList.map((item) => (item === known ? account : item));
    else driveAccountList.push(account);
    return { ...account };
  },
  drive_connect_cancel: (a) => void driveFlows.delete(str(a, "flowId")),
  drive_disconnect: (a) => {
    driveAccountList = driveAccountList.filter((account) => account.id !== str(a, "account"));
  },
};

/** open_target for a Drive file: a Google Docs file's page, or where its copy lands. */
function driveOpen(path: string): OpenTarget {
  const entry = driveNode(path);
  if (entry?.cloud?.web_url) return { kind: "url", url: entry.cloud.web_url };
  return { kind: "file", path: `/tmp/opencode/mock-drive/${entry?.name ?? "file"}` };
}

// ----- Check for Updates: tests drive the answers through __mockUpdate -----

type UpdateKind = "app_image" | "mac_app" | "linux_package" | "dev_build";
type UpdateFailure = { kind: string; detail: string };
type UpdateProgress = { phase: "downloading" | "installing"; downloaded: number; total: number | null };
type Install = { channel?: Channel<UpdateProgress>; downloaded: number; resolve: () => void; reject: (f: UpdateFailure) => void };

function freshUpdateState() {
  return {
    latest: null as string | null,
    kind: "app_image" as UpdateKind,
    failCheck: undefined as UpdateFailure | undefined,
    failInstall: undefined as UpdateFailure | undefined,
    total: 91_400_000 as number | null,
    /** Cancel arrives after installing started: the backend ignores it. */
    lateCancel: false,
    /** A copy or move runs in another window (the backend registry is app-wide). */
    otherWindowBusy: false,
    /** Another window's Download and Install is running. */
    otherWindowInstalling: false,
    /** The version installed in this session, waiting for a restart. */
    installedVersion: null as string | null,
    holding: false,
    checkHolding: false,
    checkWaiters: [] as (() => void)[],
    pending: null as string | null,
    install: null as Install | null,
    installed: 0,
    restarts: 0,
  };
}
let upd = freshUpdateState();
const updateFailed = (detail: string): UpdateFailure => ({ kind: "failed", detail });

async function checkUpdate() {
  if (upd.checkHolding) await new Promise<void>((resolve) => upd.checkWaiters.push(resolve));
  if (upd.failCheck) throw upd.failCheck;
  upd.pending = upd.latest;
  const releases = "https://github.com/ahmadsayed/unshitty-mac-filebrowser/releases";
  return {
    current: "0.5.3",
    latest: upd.latest,
    kind: upd.kind,
    release_url: upd.latest ? `${releases}/tag/v${upd.latest}` : `${releases}/latest`,
    restart_pending: upd.installedVersion !== null && upd.installedVersion === upd.latest,
  };
}

function installUpdate(channel?: Channel<UpdateProgress>): Promise<void> {
  if (upd.kind !== "app_image" && upd.kind !== "mac_app") throw updateFailed("This copy can't update itself.");
  if (!upd.pending) throw updateFailed("No update to install.");
  if (upd.install || upd.otherWindowInstalling) throw { kind: "busy", detail: "An update is already being installed." };
  return new Promise((resolve, reject) => {
    upd.install = { channel, downloaded: 0, resolve, reject };
    if (!upd.holding) finishInstall();
  });
}

function finishInstall(): void {
  const install = upd.install;
  if (!install) return;
  upd.install = null;
  if (upd.failInstall) return install.reject(upd.failInstall);
  install.channel?.onmessage({ phase: "installing", downloaded: upd.total ?? install.downloaded, total: upd.total });
  upd.installed++;
  upd.installedVersion = upd.pending;
  install.resolve();
}

(globalThis as Record<string, unknown>).__mockUpdate = {
  set(o: Partial<Pick<typeof upd, "latest" | "kind" | "failCheck" | "failInstall" | "total" | "lateCancel" | "otherWindowBusy" | "otherWindowInstalling">>) {
    Object.assign(upd, o);
  },
  hold() {
    upd.holding = true;
  },
  step(bytes: number) {
    const install = upd.install;
    if (!install) return;
    install.downloaded += bytes;
    install.channel?.onmessage({ phase: "downloading", downloaded: install.downloaded, total: upd.total });
  },
  release() {
    upd.holding = false;
    finishInstall();
  },
  holdCheck() {
    upd.checkHolding = true;
  },
  releaseCheck() {
    upd.checkHolding = false;
    upd.checkWaiters.splice(0).forEach((resolve) => resolve());
  },
  installed: () => upd.installed,
  restarts: () => upd.restarts,
  reset() {
    upd.install?.reject(updateFailed("Cancelled."));
    upd = freshUpdateState();
  },
};

// ----- Open With: tests vary the installed applications through __mockApps -----

type MockApp = { name: string; id: string };
const DEFAULT_APPS: MockApp[] = [
  { name: "TextEdit", id: "/Applications/TextEdit.app" },
  { name: "Preview", id: "/Applications/Preview.app" },
  { name: "Visual Studio Code", id: "/Applications/Visual Studio Code.app" },
];

function freshAppsState() {
  return {
    list: DEFAULT_APPS,
    /** list_apps rejects with this. */
    failList: "",
    /** The next open_with rejects with this, once. */
    failOpen: "",
    /** list_apps waits for release() while set. */
    holding: false,
    waiters: [] as (() => void)[],
    opened: [] as { path: string; app: string }[],
  };
}
let apps = freshAppsState();

(globalThis as Record<string, unknown>).__mockApps = {
  set(o: Partial<Pick<typeof apps, "list" | "failList" | "failOpen">>) {
    Object.assign(apps, o);
  },
  hold() {
    apps.holding = true;
  },
  release() {
    apps.holding = false;
    apps.waiters.splice(0).forEach((resolve) => resolve());
  },
  opened: () => apps.opened.map((o) => ({ ...o })),
  reset() {
    apps.waiters.splice(0).forEach((resolve) => resolve());
    apps = freshAppsState();
  },
};

async function mockListApps(): Promise<MockApp[]> {
  if (apps.holding) await new Promise<void>((resolve) => apps.waiters.push(resolve));
  if (apps.failList) throw apps.failList;
  return apps.list.map((app) => ({ ...app }));
}

/** Applications the Arbesk tests and screenshots add (`__mockApps.set`),
 *  named by `APP_FORMATS`; the default list stays as it was. */
export const MOCK_ARBESK_APPS: MockApp[] = [
  { name: "PrusaSlicer", id: "/Applications/PrusaSlicer.app" },
  { name: "Blender", id: "/Applications/Blender.app" },
  { name: "Text Editor", id: "/Applications/Text Editor.app" },
];

/** What each mock application declares it opens, by name; any other opens nothing. */
const APP_FORMATS: Record<string, string[]> = {
  PrusaSlicer: ["3mf", "gcode", "stl"],
  Blender: ["blend", "gltf", "glb", "stl"],
  "Text Editor": ["gcode"],
};

/** `app_formats` (spec F8): which of `formats` the application declares,
 *  each once, in order; an extension Arbesk can't record is never asked. */
function mockAppFormats(a: Args): string[] {
  const app = apps.list.find((choice) => choice.id === str(a, "app"));
  const opens = APP_FORMATS[app?.name ?? ""] ?? [];
  const formats = (a.formats as string[]).filter((format) => /^[a-z0-9]{1,16}$/.test(format) && opens.includes(format));
  return [...new Set(formats)];
}

function mockOpenWith(a: Args): undefined {
  const path = localPath(str(a, "path"));
  if (!lookup(path)) throw "No such file or directory";
  if (apps.failOpen) {
    const failure = apps.failOpen;
    apps.failOpen = "";
    throw failure;
  }
  apps.opened.push({ path, app: str(a, "app") });
  return undefined;
}

/** Every command the app calls, by name. */
/** The window the header bar buttons act on; tests read `actions`. */
const mockWindow = {
  maximized: false,
  actions: [] as string[],
  drags: [] as string[][],
  /** Set to act as if the button was released before the drag could start. */
  buttonUp: false,
  dragEnded: null as Channel<boolean> | null,
  /** GTK's drag-end for the last drag out: whether it landed on this window. */
  endDrag(landedHere = false) {
    mockWindow.dragEnded?.onmessage(landedHere);
    mockWindow.dragEnded = null;
  },
};
(globalThis as Record<string, unknown>).__mockWindow = mockWindow;

function mockDmgInspection(path: string) {
  const node = lookup(path);
  if (!node || node.dir || node.link || !/\.dmg$/i.test(path)) throw "Choose a local disk image file.";
  const name = path.slice(path.lastIndexOf("/") + 1).replace(/\.dmg$/i, "");
  if (name === "Packages") throw "No application bundles were found. Package installers are not supported by this action.";
  const names = name === "Multiple" ? ["First", "Second"] : [name];
  return { fingerprint: `mock-dmg:${path}:${node.size}:${node.modified}`, warning: null,
    apps: names.map((name) => ({ id: `${name}.app`, name, fileName: `${name}.app` })),
  };
}

const COMMANDS: Record<string, (args: Args) => unknown> = {
  inspect_dmg: (a) => mockDmgInspection(str(a, "path")),
  install_dmg: (a) => {
    const info = mockDmgInspection(str(a, "path"));
    if (a.fingerprint !== info.fingerprint) throw "The disk image changed. Read it again before installing.";
    const app = info.apps.find((app) => app.id === a.application);
    if (!app) throw "Choose an application from this disk image.";
    if (!["user", "system"].includes(str(a, "destination"))) throw "Choose a supported application folder.";
    if (!["install", "replace", "keep-both"].includes(str(a, "mode"))) throw "Choose a supported installation action.";
    const root = a.destination === "system" ? "/Applications" : "/Users/me/Applications";
    let path = `${root}/${app.fileName}`;
    if (lookup(path) && a.mode === "install") return { kind: "conflict", name: app.name, path, warning: null };
    if (lookup(path) && a.mode === "replace") trashEntry(path);
    if (a.mode === "keep-both") {
      let n = 1;
      while (lookup(path)) path = `${root}/${app.fileName.slice(0, -4)} (copy${n++ > 1 ? ` ${n - 1}` : ""}).app`;
    }
    if (!lookup(root)) makeDir(root);
    makeDir(path);
    return { kind: "installed", name: app.name, path, warning: null };
  },
  install_appimage: (a) => {
    const path = str(a, "path");
    const name = str(a, "name").trim();
    if (!name || /[\\/\u0000-\u001f\u007f]/.test(name)) throw "Enter a valid application name.";
    if (!lookup(path) || lookup(path)?.dir || lookup(path)?.link || !/\.appimage$/i.test(path)) throw "Choose a local AppImage file.";
    const id = name.toLowerCase().replace(/[^a-z0-9]/g, "-");
    const desktop = `/Users/me/.local/share/applications/${id}.desktop`;
    if (lookup(desktop)) throw "An application with that name is already installed. Choose a different name.";
    const installed = `/Users/me/.local/share/filebrowser/appimages/${id}/application.AppImage`;
    for (const dir of [`/Users/me/.local/share/applications`, `/Users/me/.local/share/filebrowser/appimages/${id}`]) {
      let part = "";
      for (const name of dir.split("/").filter(Boolean)) {
        part += `/${name}`;
        if (!lookup(part)) makeDir(part);
      }
    }
    transfer({ from: path, to: installed, overwrite: false }, false);
    const parent = lookup(`/Users/me/.local/share/applications`);
    if (parent?.dir) parent.children[`${id}.desktop`] = f(200, 0);
    return { name, path: installed, desktop };
  },
  run_shell_script: (a) => {
    const path = str(a, "path");
    if (!lookup(path) || lookup(path)?.dir || lookup(path)?.link || !/\.(sh|bash)$/i.test(path)) throw "Choose a local Bash script.";
    return undefined;
  },
  list_dir: (a) => {
    const path = str(a, "path");
    const lists = { sftp: listSftp, share: listShare, drive: listDrive, arbesk: listArbesk, local: (p: string) => list(localPath(p), a.showHidden as boolean) };
    return lists[placeOf(path)](path);
  },
  search_dir: searchCommand,
  cancel_search: () => {
    searchMock.cancels += 1;
  },
  item_counts: (a) => itemCounts(a.paths as string[], a.showHidden as boolean),
  file_properties: (a) => {
    const path = str(a, "path");
    if (placeOf(path) === "share") throw "Properties aren't available for shared folders yet.";
    if (placeOf(path) === "sftp") throw "Properties aren't available for SSH servers yet.";
    if (placeOf(path) === "arbesk") return arbeskProperties(path);
    return placeOf(path) === "drive" ? driveProperties(path) : fileProperties(localPath(path));
  },
  set_permissions: (a) => {
    const path = localWrite(str(a, "path"));
    if (!lookup(path)) throw "Failed to read permissions: No such file or directory (os error 2)";
    const mode = Number(a.mode);
    if (!Number.isInteger(mode) || mode < 0 || mode > 0o777) throw "Invalid permissions.";
    mockModes.set(path, mode);
  },
  extract_archive: (a) => {
    const path = localPath(str(a, "path"));
    const dest = localWrite(str(a, "dest"));
    const node = lookup(path);
    if (!node || node.dir) throw "The archive no longer exists.";
    if (!/\.(zip|jar|tar|tar\.gz|tgz|tar\.bz2|tbz2|tbz|tar\.xz|txz|tar\.zst|tzst|gz|bz2|xz|zst)$/i.test(path)) {
      throw "This archive format isn't supported yet.";
    }
    let target = lookup(dest);
    if (!target?.dir) {
      if (!a.create) throw "The destination folder no longer exists.";
      target = ensureDir(dest);
    }
    if (target.children["extracted.txt"]) throw "“extracted.txt” already exists.";
    target.children["extracted.txt"] = f(5, 0);
    return { path: dest, files: 1, folders: 0, skipped: 0 };
  },
  new_window: () => {
    const globals = globalThis as unknown as { __mockNewWindows?: number };
    globals.__mockNewWindows = (globals.__mockNewWindows ?? 0) + 1;
  },
  sftp_connect: (a) => {
    const input = str(a, "url").trim();
    const raw = input.includes("://") ? input : `sftp://${input}`;
    const url = new URL(raw);
    const user = url.username;
    const host = url.hostname;
    const port = url.port ? Number(url.port) : 22;
    if (!user || !host) throw "Enter the SSH user, for example user@host.";
    const id = `${user}@${host}:${port}`;
    const label = port === 22 ? `${user}@${host}` : `${user}@${host}:${port}`;
    const root = `sftp://${user}@${host}${port === 22 ? "" : `:${port}`}/`;
    const server = { id, label, url: root, host, port, user };
    if (mockSftpUnknownHost && !a.trustHost) {
      return { kind: "host-key-unknown", server, fingerprint: "SHA256:mock-fingerprint" };
    }
    if (mockSftpNeedsAuth && a.password !== "secret") {
      return { kind: "auth-needed", server };
    }
    // Connected, the server's URL is where it starts, as in sftp.rs: the
    // path typed, or the user's home on the server. Never the root alone.
    const start = { ...server, url: `${root.slice(0, -1)}${url.pathname.length > 1 ? url.pathname : `/home/${user}`}` };
    // A second connection to the server replaces the first, as in sftp.rs.
    const known = mockSftpServers.some((item) => item.id === id);
    mockSftpServers = known ? mockSftpServers.map((item) => (item.id === id ? start : item)) : [...mockSftpServers, start];
    return { kind: "connected", server: start };
  },
  sftp_servers: () => mockSftpServers,
  sftp_disconnect: (a) => {
    mockSftpServers = mockSftpServers.filter((server) => server.id !== str(a, "id"));
  },
  create_symlink: (a) => {
    const path = localWrite(str(a, "path"));
    const node = lookup(path);
    if (!node) throw "This item no longer exists.";
    const [parent, name] = parentAndName(path);
    if (!parent?.dir) throw "This item has no parent folder.";
    for (let n = 1; ; n++) {
      const label = n === 1 ? `Link to ${name}` : `Link to ${name} ${n}`;
      if (parent.children[label]) continue;
      parent.children[label] = { ...structuredClone(node), link: true };
      return `${path.slice(0, path.lastIndexOf("/"))}/${label}`;
    }
  },
  compress_archive: (a) => {
    const paths = (a.paths as string[]).map(localPath);
    const dest = localWrite(str(a, "dest"));
    if (!paths.length) throw "Choose at least one item.";
    if (!/\.(zip|tar\.gz|tar\.xz)$/i.test(dest)) throw "That archive format isn't supported yet.";
    for (const path of paths) {
      const node = lookup(path);
      if (!node) throw "An item no longer exists.";
    }
    const [parent, name] = parentAndName(dest);
    if (!parent?.dir) throw "The destination folder no longer exists.";
    if (parent.children[name]) throw `A file named “${name}” already exists.`;
    parent.children[name] = f(1024, 0);
  },
  create_file: (a) => {
    const path = localWrite(str(a, "path"));
    const [parent, name] = parentAndName(path);
    if (!parent?.dir) throw "The destination folder no longer exists.";
    if (parent.children[name]) throw "Could not create the file: File exists (os error 17)";
    parent.children[name] = f(0, 0);
  },
  list_templates: () => list("/Users/me/Templates", false),
  preview_text: (a) => {
    const path = localPath(str(a, "path"));
    const node = lookup(path);
    if (!node || node.dir) throw "This file can't be previewed as text.";
    const limit = Math.max(1, Math.min(1_048_576, Number(a.limit)));
    const text = `Preview of ${path}\n\nThis is sample text from the mock filesystem.\n`;
    return { text: text.slice(0, limit), truncated: text.length > limit };
  },
  // The real file's bytes; here a sample model for its name (./models).
  preview_bytes: (a) => {
    if (placeOf(str(a, "path")) === "arbesk") return arbeskBytes(str(a, "path"), Number(a.limit)).slice().buffer;
    const path = localPath(str(a, "path"));
    const node = lookup(path);
    if (!node) throw "Failed to open file: No such file or directory (os error 2)";
    if (node.dir) throw "Only files can be previewed.";
    if (node.size > Number(a.limit)) throw "This file is too large to preview.";
    const bytes = sampleModelBytes(path.slice(path.lastIndexOf("/") + 1)) ?? new TextEncoder().encode(`Contents of ${path}`);
    return bytes.slice().buffer;
  },
  list_apps: () => mockListApps(),
  // A Mac draws its own window buttons. Set __mockButtonLayout (GNOME's
  // button-layout, e.g. "close,minimize:maximize") before boot to try others.
  window_buttons: () => {
    const globals = globalThis as { __mockButtonLayout?: string };
    const layout = globals.__mockButtonLayout ?? (/Mac/.test(navigator.userAgent) ? "" : "appmenu:close");
    const [start, end = ""] = layout.split(":");
    const pick = (side: string) => side.split(",").filter((name) => ["minimize", "maximize", "close"].includes(name));
    return { start: pick(start), end: pick(end) };
  },
  window_control: (a) => {
    const action = str(a, "action");
    if (!["minimize", "maximize", "close", "state"].includes(action)) throw `Unknown window action “${action}”`;
    if (action === "maximize") mockWindow.maximized = !mockWindow.maximized;
    if (action !== "state") mockWindow.actions.push(action);
    return mockWindow.maximized;
  },
  // Drags out of the window (Linux): the paths of each, checked like the real
  // one (which reads nothing from disk).
  start_file_drag: (a) => {
    const paths = a.paths as string[];
    if (paths.length === 0) throw "Nothing to drag.";
    const relative = paths.find((p) => !p.startsWith("/"));
    if (relative) throw `“${relative}” is not a local file.`;
    if (mockWindow.buttonUp) return false;
    mockWindow.drags.push(paths);
    mockWindow.dragEnded = (a.onEnd as Channel<boolean> | undefined) ?? null;
    return true;
  },
  open_file: (a) => {
    const path = localPath(str(a, "path"));
    // Downloaded remote files and opened versions live outside the fake filesystem.
    const outside = path.startsWith("/tmp/") || path.startsWith(`${OPENED}/`);
    if (!outside && !lookup(path)) throw "No such file or directory";
    if (path.endsWith(".mp3")) throw "No application knows how to open this file";
    console.log("mock open_file", path);
    return undefined;
  },
  open_url: (a) => {
    // The backend's rule: https links, and the Mac's System Settings link.
    const url = str(a, "url");
    if (!/^(https:|x-apple\.systempreferences:)\S+$/i.test(url)) throw "This link can't be opened.";
    console.log("mock open_url", url);
    return undefined;
  },
  open_with: (a) => mockOpenWith(a),
  app_formats: (a) => mockAppFormats(a),
  new_folder_walk: () => ++mockWalks,
  cancel_folder_size: () => undefined,
  folder_size: (a) => {
    const path = str(a, "path");
    if (placeOf(path) === "sftp") throw "Folder sizes aren't available for SSH servers yet.";
    if (placeOf(path) === "share") throw "Folder sizes aren't available for shared folders yet.";
    const node = lookup(localPath(path));
    return node ? totalSize(node) : 0;
  },
  make_dir: (a) => {
    const path = str(a, "path");
    if (placeOf(path) === "drive") return driveMakeDir(path);
    makeDir(localWrite(path));
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
  thumbnail: async (a) => {
    const path = str(a, "path");
    if (path.startsWith("arbesk://")) return arbeskThumbnail(path, a.size as number);
    if (!isLocal(path)) return new ArrayBuffer(0);
    const ticket = a.ticket as number | undefined;
    thumbLog.requested.push(path);
    if (thumbsHeld && ticket !== undefined) {
      await new Promise<void>((resolve) => heldThumbs.set(ticket, resolve));
      // Cancelled while it waited: the backend answers with no image.
      if (thumbLog.cancelled.includes(ticket)) return new ArrayBuffer(0);
    }
    return mockThumbnail(path, a.size as number);
  },
  icon_ids: async (a) => {
    const paths = a.paths as string[];
    nativeIconMock.log.ids.push(paths);
    if (nativeIconMock.delay) await new Promise((r) => setTimeout(r, nativeIconMock.delay));
    return paths.map(mockIconId);
  },
  file_share_prepare: (a) => prepareFileShare(a.paths as string[]),
  file_share_status: (a) => fileShareStatus(str(a, "id")),
  file_share_stop: (a) => void fileShareMock.shares.delete(str(a, "id")),
  // Apps are their .app bundles, as on a Mac.
  app_icon_ids: (a) => {
    const apps = a.apps as string[];
    nativeIconMock.log.ids.push(apps.map((app) => `app:${app}`));
    return apps.map((app) => (nativeIconMock.enabled && /\.app$/.test(app) ? `path:${app}` : null));
  },
  icon_image: (a) => mockIconImage(str(a, "id"), a.size as number),
  cancel_thumbnails: (a) => {
    for (const ticket of a.tickets as number[]) {
      thumbLog.cancelled.push(ticket);
      heldThumbs.get(ticket)?.();
      heldThumbs.delete(ticket);
    }
  },
  // Shares and SSH servers have no path info and nothing "exists" there,
  // as in the backend (place::local_only).
  path_info: (a) =>
    (a.paths as string[]).map((path) => {
      if (placeOf(path) !== "drive") return isLocal(path) ? pathInfo(path) : null;
      const entry = driveNode(path);
      return entry ? { is_dir: entry.is_dir, device: 0 } : null;
    }),
  // The backend's table is its providers' own answers; the frontend's
  // built-in copy is pinned to it by tests/capabilities.json.
  capabilities: () => structuredClone(BUILT_IN_CAPABILITIES),
  path_exists: (a) => {
    const path = str(a, "path");
    if (placeOf(path) === "drive") return drivePathExists(path);
    if (placeOf(path) === "arbesk") return arbeskPendingExists(path);
    return isLocal(path) && lookup(path) !== undefined;
  },
  copy_entry: (a) => {
    if (placeOf(str(a, "from")) === "arbesk") return arbeskCopy(a);
    if (placeOf(str(a, "to")) === "arbesk") return arbeskAdd(a);
    return transferRoute(a) === "drive" ? driveTransfer(a, false) : (transfer(a, false), str(a, "to"));
  },
  move_entry: (a) => (transferRoute(a) === "drive" ? driveTransfer(a, true) : (transfer(a, true), str(a, "to"))),
  trash_entries: trashEntries,
  trash_info: (a) => (a.paths as string[]).map((path) => (isLocal(path) ? trashInfo(path) : null)),
  restore_entry: (a) => {
    const path = str(a, "path");
    return placeOf(path) === "drive" ? driveTrash(path, false) : restoreEntry(localPath(path));
  },
  delete_permanently: (a) => deletePermanently(localPath(str(a, "path"))),
  empty_trash: () => emptyTrash(),
  rename_entry: (a) => {
    const from = str(a, "from"), to = str(a, "to");
    if (transferRoute(a) === "drive") return driveRename(from, to);
    renameEntry(from, to);
    return to;
  },
  rename_many: (a) => {
    const items = a.items as Array<{ from: string; to: string }>;
    items.forEach((item) => (localWrite(item.from), localWrite(item.to)));
    return renameMany(items);
  },
  ...SHARE_COMMANDS,
  ...DRIVE_COMMANDS,
  ...ARBESK_COMMANDS,
  ...VERSION_COMMANDS,
  measure: async (a) => {
    if (measureHolding) await new Promise<void>((resolve, reject) => held.push({ op: "", bytes: 0, resolve, reject }));
    return measure(a.paths as string[]);
  },
  cancel_operation: (a) => cancelOperation(str(a, "op")),
  check_update: () => checkUpdate(),
  install_update: (a) => installUpdate(a.onProgress as Channel<UpdateProgress> | undefined),
  cancel_update: () => {
    if (upd.lateCancel) return;
    upd.install?.reject(updateFailed("Cancelled."));
    upd.install = null;
  },
  operations_running: () => upd.otherWindowBusy || held.length > 0,
  restart_app: () => {
    if (upd.otherWindowBusy || held.length > 0) throw updateFailed("Operations are still running.");
    upd.restarts++;
  },
};

// Long commands report progress and can be held by tests (see __mockOps).
for (const name of ["copy_entry", "move_entry", "delete_permanently", "empty_trash"]) {
  COMMANDS[name] = gated(COMMANDS[name]);
}

// ----- Opening a file (open_target): remote files download first -----

type OpenTarget = { kind: "file"; path: string } | { kind: "url"; url: string };

/** A remote file's copy to open, by its kind, as the backend's handlers answer. */
const DOWNLOAD_TO_OPEN: Record<Exclude<Place, "local">, (path: string) => OpenTarget> = {
  drive: driveOpen,
  share: (path) => ({ kind: "file", path: shareOpen(path) }),
  sftp: (path) => ({ kind: "file", path: `/tmp/opencode/mock-sftp/${decodeURIComponent(path.split("/").pop() ?? "file")}` }),
  // As the backend answers: the asset's Studio link, opened in the browser.
  arbesk: (path) => {
    const { token, file } = arbeskIds(path);
    if (file !== undefined) return { kind: "file", path: arbeskFileOpen(path) };
    const asset = arbeskAsset(path);
    const query = new URLSearchParams({ asset: token ?? "", assetId: asset.id });
    return { kind: "url", url: `${ARBESK_ORIGIN}/studio?${query}` };
  },
};

/** Like the backend, only a download is an operation (held, cancelled, reported). */
const downloadToOpen = gated((a) => {
  const path = str(a, "path");
  return DOWNLOAD_TO_OPEN[placeOf(path) as Exclude<Place, "local">](path);
});

COMMANDS.open_target = (a) => {
  const path = str(a, "path");
  return placeOf(path) === "local" ? { kind: "file", path: localPath(path) } : downloadToOpen(a);
};

// ----- The backend's location gate (src-tauri/src/place.rs), same wording -----

/** Throws for a path where nothing may be written: shares and SSH servers. */
function writable(path: string): void {
  if (placeOf(path) === "sftp") throw "SSH servers are read only for now.";
  if (placeOf(path) === "share") throw "Shared folders are read only.";
  if (placeOf(path) === "arbesk") throw "Arbesk is read only.";
}

/** Where a copy, move or rename runs (place::transfer): both sides
 *  writable, `from` first; a side outside Drive must be a local path. */
function transferRoute(a: Args): "drive" | "local" {
  const sides = [str(a, "from"), str(a, "to")];
  sides.forEach(writable);
  for (const side of sides) if (placeOf(side) !== "drive") localPath(side);
  return sides.some((side) => placeOf(side) === "drive") ? "drive" : "local";
}

const NOT_LOCAL: Record<Exclude<Place, "local">, string> = {
  drive: "This action is not available for Google Drive items yet.",
  share: "This action is not available for shared folders.",
  sftp: "This action is not available for SSH servers.",
  arbesk: "This action is not available for Arbesk.",
};

/** `path` when it is a file on this computer; throws why it isn't otherwise. */
function localPath(path: string): string {
  const place = placeOf(path);
  if (place !== "local") throw NOT_LOCAL[place];
  if (!path.startsWith("/")) throw `“${path}” is not a local file.`;
  return path;
}

function isLocal(path: string): boolean {
  return placeOf(path) === "local" && path.startsWith("/");
}

/** `path` when it is a file on this computer that may be written. */
function localWrite(path: string): string {
  writable(path);
  return localPath(path);
}

// ----- Any command: tests count calls, hold one until released, or fail the next (__mockCalls) -----

const calls = {
  counts: new Map<string, number>(),
  /** Commands held, each with the calls waiting for release(). */
  held: new Map<string, (() => void)[]>(),
  /** The next call of each command rejects with this, once. */
  failures: new Map<string, string>(),
};

function releaseCalls(cmd: string): void {
  const waiting = calls.held.get(cmd) ?? [];
  calls.held.delete(cmd);
  waiting.forEach((resolve) => resolve());
}

(globalThis as Record<string, unknown>).__mockCalls = {
  count: (cmd: string) => calls.counts.get(cmd) ?? 0,
  hold(cmd: string) {
    if (!calls.held.has(cmd)) calls.held.set(cmd, []);
  },
  release: releaseCalls,
  /** Release only the latest call waiting, so it lands before the earlier
   *  ones; the command stays held. */
  releaseLast(cmd: string) {
    calls.held.get(cmd)?.pop()?.();
  },
  failNext(cmd: string, error: string) {
    calls.failures.set(cmd, error);
  },
  reset() {
    [...calls.held.keys()].forEach(releaseCalls);
    calls.counts.clear();
    calls.failures.clear();
  },
};

/** Count the call, wait while the command is held, and fail it if asked. */
async function callGate(cmd: string): Promise<void> {
  calls.counts.set(cmd, (calls.counts.get(cmd) ?? 0) + 1);
  const waiting = calls.held.get(cmd);
  if (waiting) await new Promise<void>((resolve) => waiting.push(resolve));
  const failure = calls.failures.get(cmd);
  if (failure === undefined) return;
  calls.failures.delete(cmd);
  throw failure;
}

export async function invoke<T>(cmd: string, args: Args = {}): Promise<T> {
  await new Promise((r) => setTimeout(r, DELAY));
  const command = COMMANDS[cmd];
  if (!command) throw `mock: unknown command ${cmd}`;
  await callGate(cmd);
  return (await command(args)) as T;
}

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

/** Every command the app calls, by name. */
const COMMANDS: Record<string, (args: Args) => unknown> = {
  list_dir: (a) => list(str(a, "path"), a.showHidden as boolean),
  item_counts: (a) => (a.paths as string[]).map((p) => folderSummary(p, a.showHidden as boolean)),
  file_properties: (a) => fileProperties(str(a, "path")),
  new_folder_walk: () => ++mockWalks,
  cancel_folder_size: () => undefined,
  folder_size: (a) => {
    const node = lookup(str(a, "path"));
    return node ? totalSize(node) : 0;
  },
  make_dir: (a) => makeDir(str(a, "path")),
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
  thumbnail: (a) => mockThumbnail(str(a, "path"), a.size as number),
  path_info: (a) => (a.paths as string[]).map(pathInfo),
  path_exists: (a) => lookup(str(a, "path")) !== undefined,
  copy_entry: (a) => transfer(a, false),
  move_entry: (a) => transfer(a, true),
  trash_entry: (a) => trashEntry(str(a, "path")),
  trash_info: (a) => (a.paths as string[]).map(trashInfo),
  restore_entry: (a) => restoreEntry(str(a, "path")),
  delete_permanently: (a) => deletePermanently(str(a, "path")),
  empty_trash: () => emptyTrash(),
  rename_entry: (a) => renameEntry(str(a, "from"), str(a, "to")),
  ...VERSION_COMMANDS,
};

export async function invoke<T>(cmd: string, args: Args = {}): Promise<T> {
  await new Promise((r) => setTimeout(r, DELAY));
  const command = COMMANDS[cmd];
  if (!command) throw `mock: unknown command ${cmd}`;
  return (await command(args)) as T;
}

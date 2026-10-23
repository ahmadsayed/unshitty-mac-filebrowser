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
      }, 1 * day),
      Downloads: d({
        "Firefox 150.dmg": f(138_221_004, 5 * day),
        "invoice-0423.pdf": f(88_120, 2 * 3600),
        "photos-export.zip": f(712_993_112, 1 * day),
        "setup.sh": f(1_820, 20 * day),
      }, 2 * 3600),
      Movies: d({ "Holiday.mov": f(1_220_448_301, 90 * day) }),
      Music: d({ ...files("Track", 4, "flac") }),
      Pictures: d({ ...files("IMG_20", 6, "heic"), Screenshots: d(files("Screenshot", 3, "png")) }),
      Public: d(),
      "Empty folder": d({}, 30 * day),
      "a-very-long-folder-name-that-needs-to-wrap-onto-two-lines": d(),
      "main.rs": f(18_400, 10 * 60),
      "song.mp3": f(5_400_000, 6 * day),
      "link-to-docs": d({}, 50 * day, true),
      ".zshrc": f(2_048, 100 * day),
      ".config": d({ "settings.json": f(512, day) }, 10 * day),
      ".Trash": d({ "old.txt": f(42, 9 * day) }),
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

function list(path: string, showHidden: boolean): Entry[] {
  const node = lookup(path);
  if (!node) throw `Failed to read directory: No such file or directory (os error 2)`;
  if (!node.dir) throw `Failed to read directory: Not a directory (os error 20)`;
  if (path === "/System") throw `Failed to read directory: Operation not permitted (os error 1)`;
  const base = path.replace(/\/$/, "");
  return Object.entries(node.children)
    .filter(([name]) => showHidden || !name.startsWith("."))
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

function trash(parent: Node & { dir: true }, name: string): void {
  const bin = lookup("/Users/me/.Trash");
  if (!bin?.dir) throw "no trash";
  let target = name;
  for (let n = 2; bin.children[target]; n++) target = `${name} ${n}`;
  bin.children[target] = parent.children[name];
  delete parent.children[name];
}

let pins: string[] = ["/Users/me/Documents/Projects"];
const flags: Record<string, boolean> = {};

export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  await new Promise((r) => setTimeout(r, 30));
  switch (cmd) {
    case "list_dir":
      return list(args.path as string, args.showHidden as boolean) as T;
    case "item_counts":
      return (args.paths as string[]).map((p) => {
        try { return list(p, args.showHidden as boolean).length; } catch { return null; }
      }) as T;
    case "make_dir": {
      const [parent, name] = parentAndName(args.path as string);
      if (!parent?.dir) throw "Failed to create directory: No such file or directory";
      if (parent.children[name]) throw `Path already exists: ${args.path}`;
      parent.children[name] = d({}, 0);
      return undefined as T;
    }
    case "load_pins":
      return [...pins] as T;
    case "save_pins":
      pins = [...(args.paths as string[])];
      return undefined as T;
    case "get_flag":
      return (flags[args.name as string] ?? false) as T;
    case "set_flag":
      flags[args.name as string] = args.value as boolean;
      return undefined as T;
    case "has_full_disk_access":
      return false as T;
    case "path_info":
      return (args.paths as string[]).map((p) => {
        const node = lookup(p);
        return node ? { is_dir: node.dir, device: p.startsWith("/Volumes/Backup Drive") ? 2 : 1 } : null;
      }) as T;
    case "path_exists":
      return (lookup(args.path as string) !== undefined) as T;
    case "copy_entry":
    case "move_entry": {
      const from = args.from as string, to = args.to as string;
      const [fromParent, fromName] = parentAndName(from);
      const [toParent, toName] = parentAndName(to);
      const node = fromParent?.dir ? fromParent.children[fromName] : undefined;
      if (!node || !fromParent?.dir) throw `“${from}” no longer exists`;
      if (!toParent?.dir) throw "Failed to copy: No such file or directory";
      if (from === to) throw "The source and destination are the same";
      if (to.startsWith(from + "/")) throw "Cannot copy or move a folder into itself";
      if (toParent.children[toName]) {
        if (!args.overwrite) throw `Target already exists: ${to}`;
        trash(toParent, toName);
      }
      toParent.children[toName] = cmd === "copy_entry" ? structuredClone(node) : node;
      if (cmd === "move_entry") delete fromParent.children[fromName];
      return undefined as T;
    }
    case "trash_entry": {
      const [parent, name] = parentAndName(args.path as string);
      if (!parent?.dir || !parent.children[name]) throw `“${args.path}” no longer exists`;
      trash(parent, name);
      return undefined as T;
    }
    case "rename_entry": {
      const [fromParent, fromName] = parentAndName(args.from as string);
      const [toParent, toName] = parentAndName(args.to as string);
      if (!fromParent?.dir || !toParent?.dir || !fromParent.children[fromName]) throw "Failed to rename: No such file or directory";
      if (toParent.children[toName]) throw `Target already exists: ${args.to}`;
      toParent.children[toName] = fromParent.children[fromName];
      delete fromParent.children[fromName];
      return undefined as T;
    }
  }
  throw `mock: unknown command ${cmd}`;
}

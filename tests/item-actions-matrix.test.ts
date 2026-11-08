// What every kind of place offers through every surface: the item,
// background and location menus, the shortcuts that act on items (⌘C, ⌘V,
// ⌘I, ⌘D, ⌘⌫, F2), a drag, the Space preview's buttons and the list row's
// star. Written on the code before the location kinds and the shared action
// rules (src/item-actions.ts) existed, and passing on it, so the refactor
// could show it changed nothing. "Kept:" marks a difference between surfaces
// that the shared rules preserve on purpose: each is a decision for later.
import { beforeAll, beforeEach, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, closeOverlays, dialogHeading, goTo, hit, item, key, mouse, names, open, settle } from "./app";

const HOME = "/Users/me";
const TRASH = `${HOME}/.Trash`;
const HERE = `${HOME}/Matrix`;
const SOURCE = `${HOME}/MatrixSrc`;
const DRIVE = "gdrive://mock/mock-root~My%20Drive";

/** The commands an action can reach. Each fails on purpose, so nothing
 *  changes between rows: the call is all the matrix records. */
const WATCHED = ["copy_entry", "move_entry", "trash_entries", "file_properties", "save_pins", "save_starred"];
const HELD = "held by the matrix";

const calls = () =>
  (globalThis as unknown as {
    __mockCalls: { count(cmd: string): number; failNext(cmd: string, error: string): void; reset(): void };
  }).__mockCalls;

beforeAll(async () => {
  for (const dir of [HERE, `${HERE}/Sub`, SOURCE, `${HOME}/Old`, `${HOME}/Old/Inner`]) await invoke("make_dir", { path: dir });
  for (const file of [`${HERE}/note.txt`, `${SOURCE}/clip.txt`]) await invoke("create_file", { path: file });
  await invoke("trash_entries", { paths: [`${HOME}/Old`] });
  // A received share, an SSH server and a Google Drive account, all in the mock.
  await invoke("add_received", { link: `share://${"m".repeat(64)}` });
  await invoke("sftp_connect", { url: "user@host" });
  await invoke("drive_client_set", { json: JSON.stringify({ installed: { client_id: "1-a.apps.googleusercontent.com", client_secret: "s" } }) });
  const flow = await invoke<{ flow_id: string }>("drive_connect_start", {});
  await invoke("drive_connect_finish", { flowId: flow.flow_id });
  const arbesk = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: arbesk.flow_id });
  await boot();
  await key("1", { mod: true }); // the list view, whose rows have a star
});

beforeEach(closeOverlays);

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

/** The open menu's labels, with "(off)" after a disabled one. */
function menu(): string[] {
  return $$(".popover .menu-item").map((el) => `${el.textContent?.trim()}${el.hasAttribute("disabled") ? " (off)" : ""}`);
}

const sidebar = (label: string) => $$(".sidebar-row").find((r) => r.textContent?.trim() === label)!;

/** Copy SOURCE/clip.txt, so ⌘V has something to paste. */
async function armClipboard(): Promise<void> {
  // From the sidebar first: a path typed on an SSH server stays on it.
  await click(sidebar("Home"));
  await settle(10);
  await goTo(SOURCE);
  await click(hit("clip.txt"));
  await key("c", { mod: true });
}

/** Paths written to the system clipboard so far: a copy or cut writes them. */
const clipboardWrites = () => vi.mocked(navigator.clipboard.writeText).mock.calls.length;

/** What `act` did: the watched commands it called, a write to the
 *  clipboard, a rename popover, a drag (`act` returns true), a dialog, a
 *  toast (other than the failures the matrix caused). */
async function effect(act: () => Promise<unknown>): Promise<string> {
  calls().reset();
  for (const cmd of WATCHED) calls().failNext(cmd, HELD);
  $$(".toast").forEach((t) => t.remove());
  const writes = clipboardWrites();
  const dragged = await act();
  await settle(10);
  const seen = [
    ...WATCHED.filter((cmd) => calls().count(cmd) > 0),
    ...(clipboardWrites() > writes ? ["clipboard"] : []),
    ...(document.querySelector(".rename-popover") ? ["rename popover"] : []),
    ...(dragged === true ? ["drag"] : []),
    ...(dialogHeading() ? [`dialog “${dialogHeading()}”`] : []),
    ...$$(".toast .toast-title")
      .map((t) => t.textContent ?? "")
      .filter((text) => !text.includes(HELD))
      .map((text) => `toast “${text}”`),
  ];
  calls().reset();
  await closeOverlays();
  return seen.join(", ") || "nothing";
}

/** Press on `name` and move far enough to start a drag; true if one started. */
async function drag(name: string): Promise<boolean> {
  await mouse(hit(name), "mousedown", { clientX: 50, clientY: 60 });
  for (const at of [300, 301]) {
    window.dispatchEvent(new MouseEvent("mousemove", { clientX: at, clientY: at }));
    await settle(10);
  }
  const started = !!document.querySelector(".drag-ghost");
  window.dispatchEvent(new MouseEvent("mouseup", { clientX: 301, clientY: 301 }));
  await settle(10);
  return started;
}

interface Row {
  item: string[];
  background: string[];
  location: string[];
  keys: Record<"copy" | "paste" | "properties" | "bookmark" | "trash" | "rename", string>;
  drag: string;
  preview: string[];
  star: boolean;
}

/** Everything a place offers for its item `wanted` (default: the first); `go` opens the place. */
async function row(go: () => Promise<void>, wanted?: string): Promise<Row> {
  await armClipboard();
  await go();
  const name = wanted ?? names()[0];
  const select = async () => {
    await closeOverlays();
    await click(hit(name));
  };
  const opened = async (open: () => Promise<void>) => {
    await open();
    const labels = menu();
    await closeOverlays();
    return labels;
  };
  await select();
  const itemMenu = await opened(() => rightClick(item(name)));
  const background = await opened(() => rightClick($("#view")));
  const location = await opened(() => click($("#btn-location-menu")));

  const keys = {} as Row["keys"];
  keys.paste = await effect(() => key("v", { mod: true }));
  await select();
  keys.properties = await effect(() => key("i", { mod: true }));
  keys.bookmark = await effect(() => key("d", { mod: true }));
  await select();
  keys.trash = await effect(() => key("Backspace", { mod: true }));
  await select();
  keys.rename = await effect(() => key("F2"));
  await select();
  keys.copy = await effect(() => key("c", { mod: true }));
  await select();
  const dragged = await effect(() => drag(name));

  await select();
  await key(" ");
  await settle(20);
  const preview = $$("#preview-actions button").map((b) => b.textContent ?? "");
  if (!$("#preview-layer").hidden) await key("Escape");
  const star = !!item(name).querySelector("button.star");
  return { item: itemMenu, background, location, keys, drag: dragged, preview, star };
}

const place = (label: string) => async () => {
  await click(sidebar(label));
  await settle(20);
};
const fromNetwork = (name: string) => async () => {
  await place("Network")();
  await open(name);
};

const FOLDER_KEYS = {
  copy: "clipboard",
  paste: "copy_entry",
  properties: "file_properties",
  bookmark: "save_pins",
  trash: "trash_entries",
  rename: "rename popover",
};
const FOLDER_BACKGROUND = ["New Folder…", "New Document…", "Open With…", "Share This Folder…", "Paste", "Select All"];
const FOLDER_LOCATION = ["New Folder…", "New Document…", "Open With…", "Add to Bookmarks", "Share This Folder…", "Copy Location", "Paste", "Select All"];
const PSEUDO_LOCATION = ["New Folder… (off)", "Copy Location (off)", "Paste (off)", "Select All"];
const PSEUDO_KEYS = { copy: "clipboard", paste: "nothing", properties: "file_properties", bookmark: "nothing", trash: "trash_entries", rename: "nothing" };
const SHARE_COPY = "toast “Copying out of a shared folder comes next.”";
const SSH_COPY = "toast “Copying from SSH servers comes next.”";
const ARBESK_COPY = "toast “Copying from Arbesk comes next.”";
const READ_ONLY_PASTE = "toast “Shared folders are read only.”";

// prettier-ignore
const ROWS: Record<string, [go: () => Promise<void>, item: string | undefined, expected: Row]> = {
  "a file in a folder": [() => goTo(HERE), "note.txt", {
    item: ["Open", "Preview", "Open With…", "Send with QR Code…", "Cut", "Copy", "Duplicate", "Create Link", "Compress…", "Rename…", "Star", "Move to Trash", "Properties"],
    background: FOLDER_BACKGROUND,
    location: FOLDER_LOCATION,
    keys: FOLDER_KEYS,
    drag: "drag",
    preview: ["Rename…", "Move to Trash", "Open"],
    star: true,
  }],
  "a folder in a folder": [() => goTo(HERE), "Sub", {
    item: ["Open", "Preview (off)", "Open With…", "Add to Bookmarks", "Share Folder…", "Cut", "Copy", "Paste Into Folder", "Duplicate", "Create Link", "Compress…", "Rename…", "Star", "Move to Trash", "Properties"],
    background: FOLDER_BACKGROUND,
    location: FOLDER_LOCATION,
    keys: FOLDER_KEYS,
    drag: "drag",
    preview: ["Rename…", "Move to Trash", "Open"],
    star: true,
  }],
  "Recent": [place("Recent"), undefined, {
    item: ["Open", "Preview", "Open With…", "Open Item Location", "Cut", "Copy", "Unstar", "Move to Trash", "Remove from Recent", "Properties"],
    background: ["Select All"],
    location: PSEUDO_LOCATION,
    keys: PSEUDO_KEYS,
    drag: "drag",
    preview: ["Open Item Location", "Move to Trash", "Open"],
    star: true,
  }],
  "Starred": [place("Starred"), undefined, {
    item: ["Open", "Preview (off)", "Open With…", "Open Item Location", "Cut", "Copy", "Unstar", "Move to Trash", "Properties"],
    background: ["Select All"],
    location: PSEUDO_LOCATION,
    keys: PSEUDO_KEYS,
    drag: "drag",
    preview: ["Open Item Location", "Move to Trash", "Open"],
    star: true,
  }],
  "a received share in Network": [place("Network"), "Photos", {
    item: ["Open", "Copy Location", "Remove from Network"],
    background: ["Connect to Server…", "Online Accounts…", "Select All"],
    location: ["Connect to Server…", "Online Accounts…", "Reload"],
    // Kept: ⌘⌫ reaches Move to Trash, and only the backend refuses; the menu has none.
    keys: { copy: SHARE_COPY, paste: "nothing", properties: "nothing", bookmark: "nothing", trash: "trash_entries", rename: "nothing" },
    drag: SHARE_COPY,
    preview: ["Open"],
    star: false,
  }],
  "an SSH server in Network": [place("Network"), "user@host", {
    item: ["Open", "Copy Location", "Disconnect"],
    background: ["Connect to Server…", "Online Accounts…", "Select All"],
    location: ["Connect to Server…", "Online Accounts…", "Reload"],
    // Kept: as for a share in Network.
    keys: { copy: SSH_COPY, paste: "nothing", properties: "nothing", bookmark: "nothing", trash: "trash_entries", rename: "nothing" },
    drag: SSH_COPY,
    // The server is Network's own entry, though its path is where it starts
    // on the server (below the root): no Open Item Location.
    preview: ["Open"],
    star: false,
  }],
  "Google Drive in Network": [place("Network"), "Google Drive", {
    item: ["Open", "Copy Location", "Disconnect…"],
    background: ["Connect to Server…", "Online Accounts…", "Select All"],
    location: ["Connect to Server…", "Online Accounts…", "Reload"],
    // Kept: ⌘I shows My Drive's properties and ⌘⌫ reaches Move to Trash, as
    // inside Drive; the menu has neither.
    keys: { copy: "clipboard", paste: "nothing", properties: "file_properties", bookmark: "nothing", trash: "trash_entries", rename: "nothing" },
    drag: "drag",
    preview: ["Open"],
    star: false,
  }],
  "Arbesk in Network": [place("Network"), "Arbesk", {
    item: ["Open", "Disconnect…"],
    background: ["Connect to Server…", "Online Accounts…", "Select All"],
    location: ["Connect to Server…", "Online Accounts…", "Reload"],
    // The account's entry is read only like an SSH server's: no copy, no properties.
    keys: { copy: ARBESK_COPY, paste: "nothing", properties: "nothing", bookmark: "nothing", trash: "trash_entries", rename: "nothing" },
    drag: ARBESK_COPY,
    preview: ["Open"],
    star: false,
  }],
  "an Arbesk collection": [async () => { await fromNetwork("Arbesk")(); await open("Shapes"); }, "Box.gltf", {
    item: ["Open", "Copy (off)"],
    background: ["Show Hidden Files", "Reload", "Select All"],
    location: ["Reload", "Select All"],
    // ⌘D bookmarks the collection, as on an SSH server; nothing else writes.
    keys: { copy: ARBESK_COPY, paste: "toast “Arbesk is read only.”", properties: "nothing", bookmark: "save_pins", trash: "nothing", rename: "nothing" },
    drag: ARBESK_COPY,
    preview: ["Open"],
    star: false,
  }],
  "the Trash": [() => goTo(TRASH), "Old", {
    item: ["Open", "Preview (off)", "Open With…", "Restore From Trash", "Cut", "Copy", "Delete Permanently…", "Properties"],
    background: ["Empty Trash…", "Select All"],
    location: ["Empty Trash…", "Select All"],
    // ⌘D does nothing: the Trash has its own sidebar row.
    keys: { copy: "clipboard", paste: "nothing", properties: "file_properties", bookmark: "nothing", trash: "dialog “Are you sure you want to permanently delete “Old”?”", rename: "nothing" },
    drag: "drag",
    preview: ["Open"],
    star: false,
  }],
  "a folder inside the Trash": [() => goTo(`${TRASH}/Old`), "Inner", {
    item: ["Open", "Preview (off)", "Open With…", "Cut", "Copy", "Delete Permanently…", "Properties"],
    background: ["Open With…", "Select All"],
    location: ["Open With…", "Copy Location", "Select All"],
    // Kept: ⌘D bookmarks it, though its location menu has no Add to Bookmarks.
    keys: { copy: "clipboard", paste: "nothing", properties: "file_properties", bookmark: "save_pins", trash: "dialog “Are you sure you want to permanently delete “Inner”?”", rename: "nothing" },
    drag: "drag",
    preview: ["Open"],
    star: false,
  }],
  "a received share": [fromNetwork("Photos"), "hello.txt", {
    item: ["Open", "Copy (off)", "Copy Location"],
    background: ["Show Hidden Files", "Reload", "Select All"],
    location: ["Copy Location", "Reload", "Select All"],
    // Kept: ⌘D bookmarks it, though its location menu has no Add to Bookmarks.
    keys: { copy: SHARE_COPY, paste: READ_ONLY_PASTE, properties: "nothing", bookmark: "save_pins", trash: "nothing", rename: "nothing" },
    drag: SHARE_COPY,
    preview: ["Open"],
    // Kept: the row has a star; the menu has no Star.
    star: true,
  }],
  "an SSH server": [fromNetwork("user@host"), "notes.txt", {
    item: ["Open", "Copy (off)", "Copy Location"],
    background: ["Show Hidden Files", "Reload", "Select All"],
    location: ["Copy Location", "Reload", "Select All"],
    // Kept: as for a share.
    keys: { copy: SSH_COPY, paste: READ_ONLY_PASTE, properties: "nothing", bookmark: "save_pins", trash: "nothing", rename: "nothing" },
    drag: SSH_COPY,
    // The item is in the folder on screen (where the server starts): no Open Item Location.
    preview: ["Open"],
    // Kept: as for a share.
    star: true,
  }],
  "Google Drive": [() => goTo(DRIVE), "hello.txt", {
    // Kept: Open Item Location even for an item in the folder on screen.
    item: ["Open", "Open Item Location", "Cut", "Copy", "Rename…", "Move to Trash", "Properties"],
    background: ["New Folder…", "Paste", "Reload", "Select All"],
    location: ["New Folder…", "Copy Location", "Paste", "Select All"],
    // ⌘D bookmarks My Drive: it is reached from Network and has no sidebar row of its own.
    keys: { copy: "clipboard", paste: "copy_entry", properties: "file_properties", bookmark: "save_pins", trash: "trash_entries", rename: "rename popover" },
    drag: "drag",
    // Kept: no Rename… or Move to Trash, which the menu and the keys offer.
    preview: ["Open"],
    // Kept: the row has a star; the menu has no Star.
    star: true,
  }],
};

for (const [name, [go, wanted, expected]] of Object.entries(ROWS)) {
  test(`${name}: every surface`, async () => {
    expect(await row(go, wanted)).toEqual(expected);
  });
}

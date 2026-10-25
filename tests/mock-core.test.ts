// Characterization tests for the in-memory backend in dev/mock-tauri/core.ts:
// every command it serves, including its errors.
import { expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";

const HOME = "/Users/me";
type Entry = { name: string; path: string; is_dir: boolean; size: number; is_hidden: boolean };
const ls = (path: string, showHidden = false) => invoke<Entry[]>("list_dir", { path, showHidden });
const namesIn = async (path: string, showHidden = false) => (await ls(path, showHidden)).map((e) => e.name).sort();

test("list_dir lists, hides dotfiles, and reports errors like the backend", async () => {
  expect(await namesIn(`${HOME}/Desktop`)).toEqual(["todo.txt"]);
  expect(await namesIn(HOME)).not.toContain(".zshrc");
  expect(await namesIn(HOME, true)).toContain(".zshrc");
  const docs = await ls(`${HOME}/Documents`);
  expect(docs.find((e) => e.name === "Projects")).toMatchObject({ is_dir: true, size: 96, path: `${HOME}/Documents/Projects` });
  await expect(ls("/nope")).rejects.toMatch(/No such file/);
  await expect(ls(`${HOME}/main.rs`)).rejects.toMatch(/Not a directory/);
  await expect(ls("/System")).rejects.toMatch(/Operation not permitted/);
});

test("item_counts summarises folders: count, versioned, null when unreadable", async () => {
  expect(
    await invoke("item_counts", { paths: [`${HOME}/Desktop`, `${HOME}/Documents/Versioned`, "/nope"], showHidden: false }),
  ).toEqual([{ count: 1, versioned: false }, { count: 3, versioned: true }, null]);
});

test("file_properties describes files, folders and links", async () => {
  const file = await invoke<Record<string, unknown>>("file_properties", { path: `${HOME}/main.rs` });
  expect(file).toMatchObject({ name: "main.rs", parent: HOME, is_dir: false, size: 18_400, mode: "-rw-r--r--", item_count: null });
  const dir = await invoke<Record<string, unknown>>("file_properties", { path: `${HOME}/Desktop` });
  expect(dir).toMatchObject({ is_dir: true, item_count: 1, mode: "drwxr-xr-x", octal: "755" });
  const link = await invoke<Record<string, unknown>>("file_properties", { path: `${HOME}/link-to-docs` });
  expect(link).toMatchObject({ is_symlink: true, symlink_target: `${HOME}/Documents` });
  await expect(invoke("file_properties", { path: "/nope" })).rejects.toMatch(/No such file/);
});

test("folder size walks and their ids", async () => {
  const a = await invoke<number>("new_folder_walk");
  expect(await invoke<number>("new_folder_walk")).toBe(a + 1);
  expect(await invoke("cancel_folder_size", { walk: a })).toBeUndefined();
  expect(await invoke("folder_size", { path: `${HOME}/Desktop`, walk: a })).toBe(312);
  expect(await invoke("folder_size", { path: "/nope", walk: a })).toBe(0);
});

test("make_dir creates and refuses clashes and missing parents", async () => {
  await invoke("make_dir", { path: `${HOME}/Fresh` });
  expect(await namesIn(HOME)).toContain("Fresh");
  await expect(invoke("make_dir", { path: `${HOME}/Fresh` })).rejects.toMatch(/already exists/);
  await expect(invoke("make_dir", { path: "/nope/x" })).rejects.toMatch(/No such file/);
});

test("pins, flags and starred round-trip", async () => {
  expect(await invoke("load_pins")).toEqual([`${HOME}/Documents/Projects`]);
  await invoke("save_pins", { paths: [`${HOME}/Music`] });
  expect(await invoke("load_pins")).toEqual([`${HOME}/Music`]);
  expect(await invoke("get_flag", { name: "x" })).toBe(false);
  await invoke("set_flag", { name: "x", value: true });
  expect(await invoke("get_flag", { name: "x" })).toBe(true);
  await invoke("save_starred", { paths: [`${HOME}/main.rs`, `${HOME}/.zshrc`, "/nope"] });
  expect(await invoke("load_starred")).toEqual([`${HOME}/main.rs`, `${HOME}/.zshrc`, "/nope"]);
  const starred = (show: boolean) => invoke<Entry[]>("starred_files", { showHidden: show }).then((l) => l.map((e) => e.name));
  expect(await starred(false)).toEqual(["main.rs"]);
  expect(await starred(true)).toEqual(["main.rs", ".zshrc"]);
});

test("recent files are newest first, skip hidden and missing ones, and can be added and forgotten", async () => {
  const recent = (show = false) => invoke<(Entry & { used: number })[]>("recent_files", { showHidden: show });
  const first = await recent();
  expect(first[0].name).toBe("Quarterly report.pdf");
  expect(first.map((e) => e.name)).not.toContain("settings.json");
  expect((await recent(true)).map((e) => e.name)).toContain("settings.json");
  await invoke("add_recent", { path: `${HOME}/song.mp3` });
  expect((await recent())[0].name).toBe("song.mp3");
  await invoke("forget_recent", { paths: [`${HOME}/song.mp3`] });
  expect((await recent()).map((e) => e.name)).not.toContain("song.mp3");
});

test("platform answers: Full Disk Access, volumes, Trash, path info and existence", async () => {
  expect(await invoke("has_full_disk_access")).toBe(false);
  expect(await invoke("volumes")).toHaveLength(2);
  expect(await invoke("trash_dir")).toBe(`${HOME}/.Trash`);
  expect(await invoke("path_info", { paths: [`${HOME}/Desktop`, "/Volumes/Backup Drive/x", "/nope"] })).toEqual([
    { is_dir: true, device: 1 },
    null,
    null,
  ]);
  expect(await invoke("path_exists", { path: HOME })).toBe(true);
  expect(await invoke("path_exists", { path: "/nope" })).toBe(false);
});

test("thumbnails are empty for files without a preview", async () => {
  const buf = await invoke<ArrayBuffer>("thumbnail", { path: `${HOME}/main.rs`, size: 64 });
  expect(buf.byteLength).toBe(0);
});

test("copy_entry and move_entry copy, move, replace and refuse", async () => {
  await invoke("make_dir", { path: `${HOME}/Box` });
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/Box/todo.txt`, overwrite: false });
  expect(await namesIn(`${HOME}/Box`)).toEqual(["todo.txt"]);
  expect(await namesIn(`${HOME}/Desktop`)).toEqual(["todo.txt"]);
  await expect(invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/Box/todo.txt`, overwrite: false })).rejects.toMatch(/already exists/);
  // Replacing sends the old item to the Trash.
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/Box/todo.txt`, overwrite: true });
  expect(await namesIn(`${HOME}/.Trash`, true)).toContain("todo.txt");
  await invoke("move_entry", { from: `${HOME}/Box/todo.txt`, to: `${HOME}/Box/moved.txt`, overwrite: false });
  expect(await namesIn(`${HOME}/Box`)).toEqual(["moved.txt"]);
  await expect(invoke("move_entry", { from: `${HOME}/Box/gone`, to: `${HOME}/x`, overwrite: false })).rejects.toMatch(/no longer exists/);
  await expect(invoke("copy_entry", { from: `${HOME}/Box/moved.txt`, to: "/nope/x", overwrite: false })).rejects.toMatch(/No such file/);
  await expect(invoke("copy_entry", { from: `${HOME}/Box/moved.txt`, to: `${HOME}/Box/moved.txt`, overwrite: false })).rejects.toMatch(/same/);
  await expect(invoke("copy_entry", { from: `${HOME}/Box`, to: `${HOME}/Box/inner`, overwrite: false })).rejects.toMatch(/into itself/);
});

test("trash, trash info, restore, delete permanently and empty", async () => {
  await invoke("make_dir", { path: `${HOME}/Doomed` });
  const trashed = await invoke<string>("trash_entry", { path: `${HOME}/Doomed` });
  expect(trashed).toBe(`${HOME}/.Trash/Doomed`);
  await expect(invoke("trash_entry", { path: `${HOME}/Doomed` })).rejects.toMatch(/no longer exists/);
  const [info, unknown] = await invoke<{ origin: string | null }[]>("trash_info", { paths: [trashed, `${HOME}/.Trash/unknown origin.zip`] });
  expect(info.origin).toBe(`${HOME}/Doomed`);
  expect(unknown.origin).toBeNull();
  expect(await invoke("restore_entry", { path: trashed })).toBe(`${HOME}/Doomed`);
  await expect(invoke("restore_entry", { path: `${HOME}/.Trash/unknown origin.zip` })).rejects.toMatch(/unknown/);
  // Restoring onto an existing item is refused.
  const again = await invoke<string>("trash_entry", { path: `${HOME}/Doomed` });
  await invoke("make_dir", { path: `${HOME}/Doomed` });
  await expect(invoke("restore_entry", { path: again })).rejects.toMatch(/already exists/);
  await expect(invoke("delete_permanently", { path: `${HOME}/main.rs` })).rejects.toMatch(/Only items in the Trash/);
  await invoke("delete_permanently", { path: again });
  await expect(invoke("delete_permanently", { path: again })).rejects.toMatch(/no longer exists/);
  await invoke("empty_trash");
  expect(await namesIn(`${HOME}/.Trash`, true)).toEqual([]);
});

test("rename_entry renames and refuses clashes and missing items", async () => {
  await invoke("make_dir", { path: `${HOME}/Old Name` });
  await invoke("rename_entry", { from: `${HOME}/Old Name`, to: `${HOME}/New Name` });
  expect(await namesIn(HOME)).toContain("New Name");
  await expect(invoke("rename_entry", { from: `${HOME}/New Name`, to: `${HOME}/Desktop` })).rejects.toMatch(/already exists/);
  await expect(invoke("rename_entry", { from: `${HOME}/Nope`, to: `${HOME}/x` })).rejects.toMatch(/No such file/);
});

test("the Keep Versions commands", async () => {
  const V = `${HOME}/Documents/Versioned`;
  expect(await invoke("versioned_root", { path: `${V}/notes.txt` })).toBe(V);
  expect(await invoke("versioned_root", { path: `${HOME}/Documents/VersionedNot` })).toBeNull();
  expect(await namesIn(V, true)).not.toContain(".versions");
  const props = await invoke<{ item_count: number }>("file_properties", { path: V });
  expect(props.item_count).toBe(3);
  expect(await invoke<unknown[]>("list_versions", { path: `${V}/notes.txt` })).toHaveLength(4);
  expect(await invoke("version_count", { path: `${V}/image.png` })).toBe(1);
  expect(await invoke("version_count", { path: `${V}/large.txt` })).toBe(2);
  expect(await invoke("version_count", { path: `${V}/other.txt` })).toBe(0);
  const diff = (path: string, version: number) => invoke<{ kind: string }>("version_diff", { path: `${V}/${path}`, version });
  expect((await diff("image.png", 1)).kind).toBe("binary");
  expect((await diff("large.txt", 1)).kind).toBe("too_large");
  expect((await diff("notes.txt", 0)).kind).toBe("deleted");
  expect((await diff("notes.txt", 3)).kind).toBe("same");
  expect(await diff("notes.txt", 1)).toMatchObject({ kind: "text", truncated: false });
  expect(await invoke("restore_version_copy", { path: `${V}/notes.txt`, version: 1, name: "n1.txt" })).toBe(`${V}/n1.txt`);
  await expect(invoke("restore_version_copy", { path: `${V}/notes.txt`, version: 1, name: "n1.txt" })).rejects.toMatch(/already exists/);
  expect(await invoke("open_version", { path: `${V}/notes.txt`, version: 1, name: "n1.txt" })).toBe(`${HOME}/.cache/opened-versions/n1.txt`);
  await expect(invoke("restore_version_copy", { path: "/nope/x", version: 1, name: "y" })).rejects.toMatch(/missing folder/);
  await invoke("replace_with_version", { path: `${V}/notes.txt`, version: 1 });
  await expect(invoke("replace_with_version", { path: `${V}/gone.txt`, version: 1 })).rejects.toMatch(/not in the folder/);
  expect(await invoke("deleted_files", { dir: V })).toEqual([expect.objectContaining({ path: `${V}/draft.md` })]);
  expect(await invoke("open_deleted", { path: `${V}/draft.md`, name: "draft (old).md" })).toBe(
    `${HOME}/.cache/opened-versions/draft (old).md`,
  );
  expect(await invoke("restore_deleted", { path: `${V}/draft.md`, fallbackName: "" })).toBe(`${V}/draft.md`);
  await expect(invoke("restore_deleted", { path: `${V}/draft.md`, fallbackName: "" })).rejects.toMatch(/already at that path/);
  expect(await invoke("history_info", { root: V })).toEqual({ bytes: 12_400_000, versions: 214, files: 37 });
  await invoke("clear_history", { root: V });
  await invoke("make_versioned_dir", { path: `${HOME}/Kept` });
  expect(await invoke("versioned_root", { path: `${HOME}/Kept` })).toBe(`${HOME}/Kept`);
  await expect(invoke("make_versioned_dir", { path: `${HOME}/Kept` })).rejects.toMatch(/already exists/);
  await invoke("stop_keeping_versions", { root: `${HOME}/Kept` });
  expect(await invoke("versioned_root", { path: `${HOME}/Kept` })).toBeNull();
});

test("Google Drive account commands connect, cancel and disconnect", async () => {
  const accounts = () => invoke<Array<{ id: string; email: string; root_path: string }>>("drive_accounts");
  expect(await accounts()).toEqual([]);

  await expect(invoke("drive_connect_start")).rejects.toMatch(/own OAuth client/);
  await invoke("drive_client_set", { json: DESKTOP_CLIENT });

  const start = await invoke<{ flow_id: string; auth_url: string }>("drive_connect_start");
  expect(start.auth_url).toContain("accounts.google.com");
  expect(await accounts()).toEqual([]);

  const account = await invoke<{ id: string; email: string; root_path: string }>("drive_connect_finish", {
    flowId: start.flow_id,
  });
  expect(account).toMatchObject({
    email: "me@example.com",
    root_path: "gdrive://mock/mock-root~My%20Drive",
  });
  expect(await accounts()).toHaveLength(1);

  await invoke("drive_disconnect", { account: account.id });
  expect(await accounts()).toEqual([]);

  const cancelled = await invoke<{ flow_id: string }>("drive_connect_start");
  await invoke("drive_connect_cancel", { flowId: cancelled.flow_id });
  await expect(invoke("drive_connect_finish", { flowId: cancelled.flow_id })).rejects.toMatch(
    /no longer active/,
  );
});

const DESKTOP_CLIENT = JSON.stringify({
  installed: { client_id: "123-abc.apps.googleusercontent.com", client_secret: "GOCSPX-test" },
});

test("the Google OAuth client is validated and locked while connected", async () => {
  const status = () => invoke<{ configured: boolean; client_id: string | null }>("drive_client_status");
  await invoke("drive_client_clear");
  expect(await status()).toEqual({ configured: false, client_id: null });

  await expect(invoke("drive_client_set", { json: "nope" })).rejects.toMatch(/not an OAuth client/);
  await expect(invoke("drive_client_set", { json: '{"web":{}}' })).rejects.toMatch(/Desktop app/);
  await expect(
    invoke("drive_client_set", { json: '{"installed":{"client_id":"x","client_secret":"s"}}' }),
  ).rejects.toMatch(/client ID/);
  await expect(
    invoke("drive_client_set", {
      json: '{"installed":{"client_id":"1-a.apps.googleusercontent.com"}}',
    }),
  ).rejects.toMatch(/client secret/);

  expect(await invoke("drive_client_set", { json: DESKTOP_CLIENT })).toEqual({
    configured: true,
    client_id: "123-abc.apps.googleusercontent.com",
  });

  const start = await invoke<{ flow_id: string }>("drive_connect_start");
  const account = await invoke<{ id: string }>("drive_connect_finish", { flowId: start.flow_id });
  await expect(invoke("drive_client_clear")).rejects.toMatch(/Disconnect Google Drive first/);
  await expect(invoke("drive_client_set", { json: DESKTOP_CLIENT })).rejects.toMatch(/Disconnect/);
  await invoke("drive_disconnect", { account: account.id });
  await invoke("drive_client_clear");
  expect(await status()).toEqual({ configured: false, client_id: null });
});

test("write commands return the item's final path", async () => {
  expect(await invoke("make_dir", { path: `${HOME}/Final` })).toBe(`${HOME}/Final`);
  expect(await invoke("rename_entry", { from: `${HOME}/Final`, to: `${HOME}/Final2` })).toBe(`${HOME}/Final2`);
  expect(
    await invoke("copy_entry", { from: `${HOME}/Final2`, to: `${HOME}/Final3`, overwrite: false }),
  ).toBe(`${HOME}/Final3`);
  expect(
    await invoke("move_entry", { from: `${HOME}/Final3`, to: `${HOME}/Final4`, overwrite: false }),
  ).toBe(`${HOME}/Final4`);
});

const DRIVE = "gdrive://mock/mock-root~My%20Drive";
const DRIVE_DOCS = `${DRIVE}/folder-1~Documents`;
const DRIVE_HELLO = `${DRIVE}/file-1~hello.txt`;
const names = async (path: string) =>
  (await invoke<Array<{ name: string }>>("list_dir", { path, showHidden: false })).map((e) => e.name);

test("Drive New Folder, Rename and path_exists use pending paths", async () => {
  const made = await invoke<string>("make_dir", { path: `${DRIVE}/~New%20Folder` });
  expect(made).toMatch(/^gdrive:\/\/mock\/mock-root~My%20Drive\/drive-\d+~New%20Folder$/);
  expect(await invoke("path_exists", { path: `${DRIVE}/~New%20Folder` })).toBe(true);
  expect(await invoke("path_exists", { path: `${DRIVE}/~Nope` })).toBe(false);
  expect(await invoke("path_exists", { path: made })).toBe(true);
  await expect(invoke("make_dir", { path: `${DRIVE}/~New%20Folder` })).rejects.toMatch(
    "A folder with that name already exists.",
  );
  const renamed = await invoke<string>("rename_entry", { from: made, to: `${DRIVE}/~Renamed` });
  expect(renamed).toBe(made.replace("~New%20Folder", "~Renamed"));
  await expect(invoke("rename_entry", { from: renamed, to: `${DRIVE}/~hello.txt` })).rejects.toMatch(
    "A file or folder with that name already exists.",
  );
  expect(await names(DRIVE)).toContain("Renamed");
});

test("Drive copy, move, trash and restore", async () => {
  const copy = await invoke<string>("copy_entry", { from: DRIVE_HELLO, to: `${DRIVE_DOCS}/~hello.txt`, overwrite: false });
  expect(copy).toMatch(/\/drive-\d+~hello\.txt$/);
  await expect(
    invoke("copy_entry", { from: DRIVE_HELLO, to: `${DRIVE_DOCS}/~hello.txt`, overwrite: false }),
  ).rejects.toMatch("Target already exists: hello.txt");
  await expect(
    invoke("move_entry", { from: DRIVE_DOCS, to: `${DRIVE_DOCS}/~Documents`, overwrite: false }),
  ).rejects.toMatch("Cannot copy or move a folder into itself");
  const moved = await invoke<string>("move_entry", { from: copy, to: `${DRIVE}/~moved.txt`, overwrite: false });
  expect(moved).toBe(`${DRIVE}/${copy.split("/").pop()!.split("~")[0]}~moved.txt`);
  expect(await names(DRIVE_DOCS)).not.toContain("hello.txt");

  expect(await invoke("trash_entry", { path: moved })).toBe(moved);
  expect(await names(DRIVE)).not.toContain("moved.txt");
  expect(await invoke("restore_entry", { path: moved })).toBe(moved);
  expect(await names(DRIVE)).toContain("moved.txt");
  await expect(invoke("delete_permanently", { path: moved })).rejects.toMatch(/Trash/);
});

test("copying between local folders and Drive", async () => {
  const up = await invoke<string>("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${DRIVE}/~todo.txt`, overwrite: false });
  expect(up).toMatch(/~todo\.txt$/);
  expect(await names(DRIVE)).toContain("todo.txt");
  expect(await names(`${HOME}/Desktop`)).toContain("todo.txt");

  await invoke("make_dir", { path: `${HOME}/FromDrive` });
  const down = await invoke<string>("copy_entry", { from: DRIVE_DOCS, to: `${HOME}/FromDrive/Documents`, overwrite: false });
  expect(down).toBe(`${HOME}/FromDrive/Documents`);
  expect(await names(down)).toContain("nested.txt");

  await expect(
    invoke("copy_entry", { from: `${DRIVE}/file-3~Plan`, to: `${HOME}/FromDrive/Plan`, overwrite: false }),
  ).rejects.toMatch("“Plan” is a Google Docs file and can only be copied within Google Drive.");

  await invoke("move_entry", { from: up, to: `${HOME}/FromDrive/todo.txt`, overwrite: false });
  expect(await names(DRIVE)).not.toContain("todo.txt");
  expect(await names(`${HOME}/FromDrive`)).toContain("todo.txt");
});

test("the mock Drive refuses like the backend on failure paths", async () => {
  await expect(
    invoke("copy_entry", { from: `${DRIVE}/file-2~Report.docx`, to: `${DRIVE}/~Report.docx`, overwrite: true }),
  ).rejects.toMatch("The source and destination are the same");

  await invoke("copy_entry", { from: `${DRIVE}/file-3~Plan`, to: `${DRIVE_DOCS}/~Plan`, overwrite: false });
  await invoke("make_dir", { path: `${HOME}/Into` });
  await expect(
    invoke("move_entry", { from: DRIVE_DOCS, to: `${HOME}/Into/Documents`, overwrite: false }),
  ).rejects.toMatch("“Plan” is a Google Docs file and can only be copied within Google Drive.");
  expect(await names(`${HOME}/Into/Documents`)).toContain("nested.txt");
  expect(await names(DRIVE)).toContain("Documents");
});

test("extract_archive refuses to overwrite and rejects unknown formats", async () => {
  const first = await invoke<Record<string, unknown>>("extract_archive", {
    path: `${HOME}/Downloads/photos-export.zip`,
    dest: `${HOME}/Downloads/Extracted`,
    create: true,
  });
  expect(first).toMatchObject({ files: 1, folders: 0, skipped: 0 });
  await expect(
    invoke("extract_archive", {
      path: `${HOME}/Downloads/photos-export.zip`,
      dest: `${HOME}/Downloads/Extracted`,
      create: true,
    })
  ).rejects.toMatch(/already exists/);

  await invoke("copy_entry", {
    from: `${HOME}/Downloads/photos-export.zip`,
    to: `${HOME}/Downloads/unknown.7z`,
    overwrite: false,
  });
  await expect(
    invoke("extract_archive", {
      path: `${HOME}/Downloads/unknown.7z`,
      dest: `${HOME}/Downloads/Extracted`,
      create: true,
    })
  ).rejects.toMatch(/isn't supported/);
});

test("permissions can be read and changed", async () => {  const path = `${HOME}/Desktop/todo.txt`;
  await invoke("set_permissions", { path, mode: 0o600 });
  const props = await invoke<Record<string, unknown>>("file_properties", { path, showHidden: false });
  expect(props).toMatchObject({ octal: "600", mode: "-rw-------", others: "None" });
  await expect(invoke("set_permissions", { path, mode: 0o1000 })).rejects.toMatch(/Invalid permissions/);
});

test("search_dir walks subfolders and respects hidden files", async () => {
  const found = await invoke<Array<{ name: string; path: string }>>("search_dir", {
    path: HOME,
    query: "README",
    showHidden: false,
    limit: 20,
  });
  expect(found.map((entry) => entry.path)).toContain(
    `${HOME}/Documents/Projects/unshitty-mac-filebrowser/README.md`,
  );

  const hidden = await invoke<Array<{ name: string }>>("search_dir", {
    path: HOME,
    query: ".DS_Store",
    showHidden: false,
    limit: 20,
  });
  expect(hidden).toHaveLength(0);
});

test("sharing commands create links, receive, go offline and clean up", async () => {  const mock = (globalThis as unknown as { __mockShare: { setOnline(v: boolean): void; reset(): void } }).__mockShare;
  mock.reset();
  expect(await invoke("share_identity")).toEqual({ name: null });
  await invoke("set_share_name", { name: "Ahmed" });
  expect(await invoke("share_identity")).toEqual({ name: "Ahmed" });

  const info = await invoke<Record<string, any>>("share_folder", { path: `${HOME}/Documents` });
  expect(info).toMatchObject({ status: "ok", access: [] });
  expect(await invoke<unknown[]>("list_shares")).toHaveLength(1);

  const anyone = await invoke<Record<string, any>>("create_link", {
    share: info.id,
    kind: "anyone",
  });
  expect(anyone.link).toMatch(/^share:\/\/[a-z2-7]{64,}$/);
  expect(anyone.entry).toMatchObject({ kind: "anyone", state: "unused" });
  const person = await invoke<Record<string, any>>("create_link", {
    share: info.id,
    kind: "person",
    person: "Sara",
  });
  expect(person.entry).toMatchObject({ kind: "person", state: "unused", name: "Sara" });
  expect(await invoke("share_link", { share: info.id, entry: person.entry.id })).toBe(person.link);

  await expect(invoke("add_received", { link: "hello" })).rejects.toMatch(/share link/);
  await expect(invoke("add_received", { link: anyone.link })).rejects.toMatch(/your own shared folder/);
  const received = await invoke<Record<string, any>>("add_received", { link: "share://mockreceived" });
  expect(received).toMatchObject({ folder: "Photos", sharer: "Ahmed", status: "online" });
  // The same ticket in the older fbshare1: form is the same share.
  await expect(invoke("add_received", { link: "fbshare1:mockreceived" })).rejects.toMatch(/already in Network/);
  expect(await invoke<unknown[]>("list_received")).toHaveLength(1);

  const root = await invoke<Array<{ name: string; path: string }>>("list_dir", {
    path: `share://${received.id}`,
  });
  expect(root.map((entry) => entry.name)).toEqual(["hello.txt", "Photos"]);
  expect(await invoke("share_open", { path: `share://${received.id}/hello.txt` })).toMatch(
    /mock-share\/hello.txt$/,
  );

  mock.setOnline(false);
  expect(await invoke<Array<{ status: string }>>("list_received")).toEqual([
    expect.objectContaining({ status: "offline" }),
  ]);
  mock.setOnline(true);

  await invoke("remove_access", { share: info.id, entry: person.entry.id });
  expect(await invoke<Record<string, any>>("share_info", { path: `${HOME}/Documents` })).toMatchObject({
    access: [expect.objectContaining({ id: anyone.entry.id })],
  });
  await invoke("stop_sharing", { share: info.id });
  expect(await invoke<unknown[]>("list_shares")).toEqual([]);
  await invoke("remove_received", { id: received.id });
  expect(await invoke<unknown[]>("list_received")).toEqual([]);
  mock.reset();
});

test("unknown commands are reported", async () => {
  await expect(invoke("nope")).rejects.toMatch(/unknown command nope/);
});

// Shared setup and drivers for the remembered-paths tests: the folder tree
// and the helpers that move, rename and star items through the real app.
import { invoke } from "../../dev/mock-tauri/core";
import { $, boot, click, closeOverlays, hit, item, key, mouse, settle, type } from "../app";

export const ROOT = "/Users/me/Refs";
export const DEST = `${ROOT}/Dest`;
/** Where the tests of a paste still running work, so ROOT fits on screen. */
export const RUN = "/Users/me/Running";

export type MockOps = { hold(): void; held(): number; release(): void; reset(): void };
export const ops = () => (globalThis as unknown as { __mockOps: MockOps }).__mockOps;

export async function setupRefs(): Promise<void> {
  for (const dir of ["", "/Dest", "/Other", "/A", "/Clash", "/Trashy", "/Empty", "/Marked", "/Outer", "/Outer/In", "/Cuts", "/CutDest", "/CopySrc", "/CopyDest", "/Spare", "/Halt", "/HaltDest", "/HaltNext", "/TabDir", "/Box", "/Box/Sub", "/Hist", "/Fwd"]) {
    await invoke("make_dir", { path: ROOT + dir });
  }
  for (const dir of ["", "/Wait", "/Wait/Folder", "/WaitDest", "/Race", "/Race/Folder", "/RaceDest"]) {
    await invoke("make_dir", { path: RUN + dir });
  }
  for (const file of ["/note.txt", "/A/inner.txt", "/keep.txt", "/Dest/keep.txt", "/Trashy/x.txt", "/Trashy/y.txt", "/Trashy/z.txt", "/Cuts/c.txt", "/CopySrc/f.txt", "/Halt/p.txt", "/Halt/q.txt", "/HaltDest/q.txt", "/TabDir/t.txt", "/Box/Sub/s.txt"]) {
    await invoke("create_file", { path: ROOT + file });
  }
  for (const file of ["/Wait/a.txt", "/Wait/b.txt", "/WaitDest/b.txt", "/Race/a.txt", "/Race/b.txt"]) {
    await invoke("create_file", { path: RUN + file });
  }
  await boot();
  await key("1", { mod: true }); // list view: rows have a star button
}

export async function cleanRefs(): Promise<void> {
  ops().release();
  ops().reset();
  await settle(20);
  await closeOverlays();
}

export const starred = (name: string) => item(name).querySelector(".star")!.classList.contains("starred");
export const saved = () => invoke<string[]>("load_starred");
export const exists = (path: string) => invoke<boolean>("path_exists", { path });

export async function star(name: string): Promise<void> {
  await mouse(item(name).querySelector(".star")!, "mousedown");
  await settle(10);
}

/** Select `list` (the first plainly, the rest added) in the folder on screen. */
export async function select(...list: string[]): Promise<void> {
  await click(hit(list[0]));
  for (const name of list.slice(1)) await click(hit(name), { metaKey: true });
}

/** F2 on `name`, type `to`, Enter. */
export async function rename(name: string, to: string): Promise<void> {
  await select(name);
  await key("F2");
  const input = $<HTMLInputElement>(".rename-popover input");
  await type(input, to);
  await key("Enter", { target: input });
  await settle(20);
}

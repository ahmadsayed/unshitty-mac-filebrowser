import { invoke } from "@tauri-apps/api/core";

export interface Entry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: number; // unix seconds
  is_hidden: boolean;
  is_symlink: boolean;
}

/**
 * List the contents of a directory.
 * Tauri v2 maps camelCase JS args to snake_case Rust params automatically.
 */
export async function listDir(
  path: string,
  showHidden: boolean
): Promise<Entry[]> {
  return invoke<Entry[]>("list_dir", { path, showHidden });
}

/** Create a new directory at the given path. Errors if the path already exists. */
export async function makeDir(path: string): Promise<void> {
  return invoke<void>("make_dir", { path });
}

/** Rename/move `from` to `to`. Errors if `to` already exists. */
export async function renameEntry(from: string, to: string): Promise<void> {
  return invoke<void>("rename_entry", { from, to });
}

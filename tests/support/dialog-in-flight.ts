// Shared helpers for the dialog tests: the folder each dialog works in, the
// mock's held/failed commands, and the setup every file reuses.
import { invoke } from "../../dev/mock-tauri/core";
import { $, $$, boot, closeOverlays, goTo, key } from "../app";

export const HOME = "/Users/me";
export const ROOT = `${HOME}/InFlight`;

export type MockCalls = {
  count(cmd: string): number; hold(cmd: string): void; release(cmd: string): void;
  failNext(cmd: string, error: string): void; reset(): void;
};
export const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
export const sftp = () => (globalThis as unknown as { __mockSftp: { reset(): void; requirePassword(): void; unknownHost(): void } }).__mockSftp;

export const button = (label: string) =>
  $$(".dialog-layer .dialog-responses button").find((b) => b.textContent?.trim() === label) as HTMLButtonElement;
export const message = () => $(".dialog-layer .field-message").textContent;

let folders = 0;
/** A fresh folder holding `files`, shown in the view. */
export async function folder(name: string, files: string[] = ["a.txt", "b.txt"]): Promise<string> {
  const dir = `${ROOT}/${name}-${++folders}`;
  await invoke("make_dir", { path: dir });
  for (const file of files) await invoke("create_file", { path: `${dir}/${file}` });
  await goTo(dir);
  return dir;
}

export async function setupDialogs(): Promise<void> {
  await invoke("make_dir", { path: ROOT });
  await boot();
  await key("1", { mod: true });
}

export async function cleanDialogs(): Promise<void> {
  calls().reset();
  await closeOverlays();
  if (!$("#preview-layer").hidden) await key("Escape");
}

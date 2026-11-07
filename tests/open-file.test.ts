// openFile (src/api.ts) against the mock backend: one open_target call
// says what to open, whatever kind of place the path is in.
import { expect, test, vi } from "vitest";
import { openFile } from "../src/api";

type MockCalls = { count(cmd: string): number; reset(): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;

const DRIVE = "gdrive://mock/mock-root~My%20Drive";

test("openFile opens a local file as it is, a downloaded copy of a remote one, and a Google Docs file's page", async () => {
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  calls().reset();
  await openFile("/Users/me/main.rs");
  await openFile(`${DRIVE}/file-1~hello.txt`, { op: "op-1" });
  await openFile(`${DRIVE}/file-3~Plan`, { op: "op-2" });
  await openFile("share://mockreceived/hello.txt", { op: "op-3" });
  await openFile("sftp://me@host/home/me/notes.txt", { op: "op-4" });
  expect(log.mock.calls).toEqual([
    ["mock open_file", "/Users/me/main.rs"],
    ["mock open_file", "/tmp/opencode/mock-drive/hello.txt"],
    ["mock open_url", "https://docs.google.com/document/d/mock"],
    ["mock open_file", "/tmp/opencode/mock-share/hello.txt"],
    ["mock open_file", "/tmp/opencode/mock-sftp/notes.txt"],
  ]);
  expect(calls().count("open_target")).toBe(5);
  log.mockRestore();
});

test("openFile refuses a path that isn't on this computer or in a known place", async () => {
  await expect(openFile("main.rs")).rejects.toBe("“main.rs” is not a local file.");
});

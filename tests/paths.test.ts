// Characterization tests for the path-bar rules, including the Drive cases
// that must not fall back to a local path.
import { expect, test } from "vitest";
import { resolveTypedPath } from "../src/paths";

const HOME = "/Users/me";
const CLOUD = "gdrive://acct/root~My%20Drive";

test("local paths resolve as before", () => {
  expect(resolveTypedPath("Documents", HOME, HOME)).toEqual({ path: `${HOME}/Documents` });
  expect(resolveTypedPath("/tmp/file", HOME, HOME)).toEqual({ path: "/tmp/file" });
  expect(resolveTypedPath("~/Notes", "/", HOME)).toEqual({ path: `${HOME}/Notes` });
  expect(resolveTypedPath("Documents/", HOME, HOME)).toEqual({ path: `${HOME}/Documents` });
  expect(resolveTypedPath("   ", HOME, HOME)).toEqual({ path: "" });
  expect(resolveTypedPath("Documents", "pseudo:recent", HOME)).toEqual({ path: "Documents" });
});

test("Drive URIs stay Drive URIs", () => {
  expect(resolveTypedPath(CLOUD, HOME, HOME)).toEqual({ path: CLOUD });
  expect(resolveTypedPath(`${CLOUD}/f1~Documents/`, HOME, HOME)).toEqual({
    path: `${CLOUD}/f1~Documents`,
  });
});

test("pasted Drive links become locations for the current account", () => {
  expect(
    resolveTypedPath("https://drive.google.com/file/d/abc123/view?usp=sharing", CLOUD, HOME)
  ).toEqual({ path: "gdrive://acct/abc123" });
  expect(
    resolveTypedPath("https://drive.google.com/drive/u/0/folders/folder-9", CLOUD, HOME)
  ).toEqual({ path: "gdrive://acct/folder-9" });
});

test("a bare name inside Drive is refused, but an absolute path is allowed", () => {
  expect(resolveTypedPath("Documents", CLOUD, HOME)).toEqual({
    error: "Enter a Google Drive location or a full path.",
  });
  expect(resolveTypedPath("/tmp/local", CLOUD, HOME)).toEqual({ path: "/tmp/local" });
  expect(resolveTypedPath("https://example.com/x", CLOUD, HOME)).toEqual({
    error: "Enter a Google Drive location or a full path.",
  });
});

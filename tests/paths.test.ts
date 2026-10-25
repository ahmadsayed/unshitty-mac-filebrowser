// Characterization tests for the path-bar rules, including the Drive cases
// that must not fall back to a local path.
import { expect, test } from "vitest";
import { joinCloudPath, joinSharePath, joinSftpPath, isCloudPath, isRemotePath, isShareLink, shareTicket, isSharePath, isSftpPath, resolveTypedPath } from "../src/paths";

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

test("share paths are remote but not Drive", () => {
  expect(isSharePath("share://r1")).toBe(true);
  expect(isSharePath("share://r1/sub")).toBe(true);
  expect(isSharePath("gdrive://acct/root")).toBe(false);
  expect(isRemotePath("share://r1")).toBe(true);
  expect(isRemotePath("gdrive://acct/root")).toBe(true);
  expect(isRemotePath("/Users/me")).toBe(false);
  expect(isCloudPath("share://r1")).toBe(false);
});

test("joinSharePath encodes names like Drive", () => {
  expect(joinSharePath("share://r1", "a b")).toBe("share://r1/a%20b");
  expect(joinSharePath("share://r1/", "a~b")).toBe("share://r1/a%7Eb");
});

test("share links are recognised with or without angle brackets", () => {
  const ticket = "aeu73ziw".repeat(10);
  expect(isShareLink(`share://${ticket}`)).toBe(true);
  expect(isShareLink(`  <SHARE://${ticket.toUpperCase()}>  `)).toBe(true);
  expect(isShareLink(`share://${ticket}/`)).toBe(true);
  expect(isShareLink(`/Users/me/share://${ticket}`)).toBe(false);
});

test("received-share locations are not links", () => {
  expect(isShareLink("share://r1")).toBe(false);
  expect(isShareLink("share://0123456789abcdef")).toBe(false);
  expect(isShareLink("share://0123456789abcdef/Photos")).toBe(false);
  expect(isShareLink(`share://${"a".repeat(70)}/Photos`)).toBe(false);
  expect(isShareLink("share://")).toBe(false);
});

test("a link's ticket is the same in either form", () => {
  const ticket = "aeu73ziw".repeat(10);
  expect(shareTicket(`share://${ticket}`)).toBe(ticket);
  expect(shareTicket(`<SHARE://${ticket.toUpperCase()}/>`)).toBe(ticket);
  expect(shareTicket(`fbshare1:${ticket}`)).toBeNull();
  expect(shareTicket("share://0123456789abcdef")).toBeNull();
  expect(shareTicket("hello")).toBeNull();
});

test("SFTP paths are remote and resolve as absolute locations", () => {
  expect(isSftpPath("sftp://me@host/")).toBe(true);
  expect(isSftpPath("share://r1")).toBe(false);
  expect(isRemotePath("sftp://me@host/")).toBe(true);
  expect(joinSftpPath("sftp://me@host/", "a b")).toBe("sftp://me@host/a%20b");
  expect(resolveTypedPath("sftp://me@host/docs", HOME, HOME)).toEqual({
    path: "sftp://me@host/docs",
  });
  expect(resolveTypedPath("docs", "sftp://me@host/", HOME)).toEqual({
    path: "sftp://me@host/docs",
  });
  expect(resolveTypedPath("/var/log", "sftp://me@host/", HOME)).toEqual({
    path: "sftp://me@host/var/log",
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

test("Drive child paths encode the name like the backend", () => {
  expect(joinCloudPath("gdrive://a/root~My%20Drive", "Q3 ~ draft (v2).txt")).toBe(
    "gdrive://a/root~My%20Drive/~Q3%20%7E%20draft%20%28v2%29.txt",
  );
  expect(joinCloudPath("gdrive://a/root~My%20Drive/", "é")).toBe("gdrive://a/root~My%20Drive/~%C3%A9");
  expect(joinCloudPath("gdrive://a/root~My%20Drive", "a-b_c.d")).toBe("gdrive://a/root~My%20Drive/~a-b_c.d");
});

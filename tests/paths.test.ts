// Characterization tests for the path-bar rules, including the Drive cases
// that must not fall back to a local path.
import { expect, test } from "vitest";
import {
  childPath, cloudSegmentName, decodeSegment, homeFolder, isCloudRoot, isKnownPath, isUnder, isWithin, joinCloudPath, joinSharePath, joinSftpPath,
  isCloudPath, isRemotePath, isShareLink, localSegments, parentPath, pathName, pathSegments, placeOf, shareId, shareTicket,
  isSharePath, isSftpPath, resolveTypedPath, isArbeskPath, isArbeskAccount, isArbeskAsset, isArbeskCollection, isArbeskFile, arbeskFilePin, arbeskAccountKey,
} from "../src/paths";

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
  // A server's root keeps its slash however it was typed.
  expect(resolveTypedPath("/", "sftp://me@host/docs", HOME)).toEqual({ path: "sftp://me@host/" });
  expect(resolveTypedPath("sftp://me@host", HOME, HOME)).toEqual({ path: "sftp://me@host/" });
  expect(resolveTypedPath("sftp://me@host//", HOME, HOME)).toEqual({ path: "sftp://me@host/" });
  expect(resolveTypedPath("sftp://me@host/docs/", HOME, HOME)).toEqual({ path: "sftp://me@host/docs" });
  // Without a server there is no root to keep: the path stays as typed.
  expect(resolveTypedPath("sftp:///docs", HOME, HOME)).toEqual({ path: "sftp:///docs" });
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

// ---- Each scheme's layout ---------------------------------------------------
// One row per kind of path.

const DRIVE_ROOT = "gdrive://acct/root~My%20Drive";
const NETWORK = null; // what parentPath returns above a share or server root

test.each([
  ["gdrive://acct/root~My%20Drive", "drive"],
  ["share://r1/Photos", "share"],
  ["sftp://me@host/", "sftp"],
  ["arbesk://0x00000000000000000000000000000000000000ab/1~Set", "arbesk"],
  ["/Users/me", "local"],
  ["pseudo:recent", "local"],
  ["", "local"],
])("placeOf(%j) is %j", (path, place) => {
  expect(placeOf(path)).toBe(place);
});

// prettier-ignore
test.each([
  // Drive: the name after the id's `~`, decoded; the account root has no `~`.
  [DRIVE_ROOT,                               "My Drive"],
  [`${DRIVE_ROOT}/f1~Q3%20report/`,          "Q3 report"],
  [`${DRIVE_ROOT}/~pending%20name`,          "pending name"],
  [`${DRIVE_ROOT}/f1~bad%E0`,                "bad%E0"],
  ["gdrive://acct",                          "acct"],
  // Shares: the last segment decoded, the id at the root.
  ["share://r1",                             "r1"],
  ["share://r1/Photos/a%20b",                "a b"],
  ["share://r1/%zz",                         "%zz"],
  ["share://",                               ""],
  // SSH: the authority at the root, then the last segment decoded.
  ["sftp://me@host/",                        "me@host"],
  ["sftp://me%40work@host:2222",             "me@work@host:2222"],
  ["sftp://me@host/docs/a%20b",              "a b"],
  ["sftp://me@host/docs/%zz",                "%zz"],
  ["sftp://%E0%A4%A/",                       "%E0%A4%A"],
  ["sftp://",                                ""],
  // Arbesk: the name after the id's `~`, decoded; the address at the root.
  ["arbesk://0x00000000000000000000000000000000000000ab",                           "0x00000000000000000000000000000000000000ab"],
  ["arbesk://0x00000000000000000000000000000000000000ab/12~My%20Set",               "My Set"],
  ["arbesk://0x00000000000000000000000000000000000000ab/12~My%20Set/a1~Box.gltf",   "Box.gltf"],
  // Local: the last segment as it is, none at the root.
  ["/Users/me/Documents/",                   "Documents"],
  ["/Users/me/a%20b",                        "a%20b"],
  ["/",                                      ""],
])("pathName(%j) is %j", (path, name) => {
  expect(pathName(path)).toBe(name);
});

// prettier-ignore
test.each([
  // A Drive account's first folder is its own parent.
  [DRIVE_ROOT,                               "gdrive://acct"],
  [`${DRIVE_ROOT}/f1~Docs/`,                 DRIVE_ROOT],
  ["gdrive://acct",                          "gdrive://acct"],
  ["share://r1/Photos/2024",                 "share://r1/Photos"],
  ["share://r1/Photos",                      "share://r1"],
  ["share://r1",                             NETWORK],
  ["sftp://me@host/",                        NETWORK],
  // A server's root is written with its slash, as the server's URL is.
  ["sftp://me@host/docs",                    "sftp://me@host/"],
  ["sftp://me@host/docs/",                   "sftp://me@host/"],
  ["sftp://me@host",                         NETWORK],
  ["sftp://me@host/docs/sub/",               "sftp://me@host/docs"],
  // An Arbesk account is the top: Network is above it.
  ["arbesk://0x00000000000000000000000000000000000000ab/12~Set/a1~Box.gltf",        "arbesk://0x00000000000000000000000000000000000000ab/12~Set"],
  ["arbesk://0x00000000000000000000000000000000000000ab/12~Set",                    "arbesk://0x00000000000000000000000000000000000000ab"],
  ["arbesk://0x00000000000000000000000000000000000000ab",                           NETWORK],
  ["/Users/me/Documents/",                   "/Users/me"],
  ["/Users",                                 "/"],
  ["/",                                      "/"],
  ["pseudo:recent",                          "pseudo:recent"],
])("parentPath(%j) is %j", (path, parent) => {
  expect(parentPath(path)).toBe(parent);
});

// prettier-ignore
test.each([
  [DRIVE_ROOT,                 "a b",     `${DRIVE_ROOT}/~a%20b`],
  ["sftp://me@host/",          "a b",     "sftp://me@host/a%20b"],
  ["sftp://me@host/docs",      "~x",      "sftp://me@host/docs/%7Ex"],
  // A share child is encoded as share/location.rs decodes it (NAME_SET).
  ["share://r1",               "a b",     "share://r1/a%20b"],
  ["share://r1/Photos/",       "~x é",    "share://r1/Photos/%7Ex%20%C3%A9"],
  // Local joins are byte-identical to what every copy, move and rename used.
  ["/Users/me",                "a b",     "/Users/me/a b"],
  ["/Users/me//",              "x",       "/Users/me/x"],
  ["/",                        "x",       "/x"],
  ["",                         "x",       "/x"],
  ["/Users/me",                "",        "/Users/me/"],
  ["/Users/me",                "a/b",     "/Users/me/a/b"],
])("childPath(%j, %j) is %j", (dir, name, path) => {
  expect(childPath(dir, name)).toBe(path);
});

test("pathSegments breaks a Drive path into the account and its folders", () => {
  expect(pathSegments(`${DRIVE_ROOT}/f1~Q3%20report`)).toEqual({
    root: { kind: "drive", key: "acct", path: "gdrive://acct", name: "acct" },
    segments: [
      { name: "My Drive", path: DRIVE_ROOT },
      { name: "Q3 report", path: `${DRIVE_ROOT}/f1~Q3%20report` },
    ],
  });
  expect(pathSegments("gdrive://acct").segments).toEqual([]);
});

test("pathSegments breaks a share path into the share and its folders", () => {
  expect(pathSegments("share://r1/Photos/a%20b/%zz/")).toEqual({
    root: { kind: "share", key: "r1", path: "share://r1", name: "r1" },
    segments: [
      { name: "Photos", path: "share://r1/Photos" },
      { name: "a b", path: "share://r1/Photos/a%20b" },
      { name: "%zz", path: "share://r1/Photos/a%20b/%zz" },
    ],
  });
  expect(pathSegments("share://").root.key).toBe("");
});

test("pathSegments breaks an SSH path into the server and its folders", () => {
  expect(pathSegments("sftp://me@host/docs/a%20b")).toEqual({
    root: { kind: "sftp", key: "me@host", path: "sftp://me@host/", name: "me@host" },
    segments: [
      { name: "docs", path: "sftp://me@host/docs" },
      { name: "a b", path: "sftp://me@host/docs/a%20b" },
    ],
  });
  expect(pathSegments("sftp://me%40work@host/").root.name).toBe("me@work@host");
  // A malformed authority is shown as it is, like every other segment.
  expect(pathSegments("sftp://%E0%A4%A/").root).toEqual({
    kind: "sftp", key: "%E0%A4%A", path: "sftp://%E0%A4%A/", name: "%E0%A4%A",
  });
});

test("pathSegments breaks an Arbesk path into the account and its collection", () => {
  expect(pathSegments("arbesk://0x00000000000000000000000000000000000000ab/12~My%20Set/a1~Box.gltf")).toEqual({
    root: { kind: "arbesk", key: "0x00000000000000000000000000000000000000ab", path: "arbesk://0x00000000000000000000000000000000000000ab", name: "0x00000000000000000000000000000000000000ab" },
    segments: [
      { name: "My Set", path: "arbesk://0x00000000000000000000000000000000000000ab/12~My%20Set" },
      { name: "Box.gltf", path: "arbesk://0x00000000000000000000000000000000000000ab/12~My%20Set/a1~Box.gltf" },
    ],
  });
});

test("a typed Arbesk location stays one, and a bare name inside Arbesk is refused", () => {
  const collection = "arbesk://0x00000000000000000000000000000000000000ab/1~Set";
  expect(resolveTypedPath(`${collection}/`, HOME, HOME)).toEqual({ path: collection });
  expect(resolveTypedPath(collection, "pseudo:network", HOME)).toEqual({ path: collection });
  expect(resolveTypedPath("Box.gltf", collection, HOME)).toEqual({ error: "Enter an Arbesk location or a full path." });
  expect(resolveTypedPath("/tmp", collection, HOME)).toEqual({ path: "/tmp" });
  expect(resolveTypedPath(CLOUD, collection, HOME)).toEqual({ path: CLOUD });
  expect(resolveTypedPath("sftp://me@host/docs", collection, HOME)).toEqual({ path: "sftp://me@host/docs" });
  expect(resolveTypedPath("~/Notes", collection, HOME)).toEqual({ path: `${HOME}/Notes` });
});

test("Arbesk paths are remote, keyed by their address", () => {
  expect(isArbeskPath("arbesk://0x00000000000000000000000000000000000000ab")).toBe(true);
  expect(isArbeskPath("/arbesk://x")).toBe(false);
  expect(isRemotePath("arbesk://0x00000000000000000000000000000000000000ab/1~S")).toBe(true);
  expect(arbeskAccountKey("arbesk://0x00000000000000000000000000000000000000ab/1~S/a~B.gltf")).toBe("0x00000000000000000000000000000000000000ab");
  expect(isKnownPath("arbesk://0x00000000000000000000000000000000000000ab")).toBe(true);
});

test("an Arbesk account is the top, where collections are created", () => {
  const account = "arbesk://0x00000000000000000000000000000000000000ab";
  expect(isArbeskAccount(account)).toBe(true);
  expect(isArbeskAccount(`${account}/`)).toBe(true);
  for (const path of [`${account}/1~S`, `${account}/1~S/a~B.gltf`, "/Users/me/1~S", "gdrive://acct/1~S", "arbesk://"]) {
    expect(isArbeskAccount(path), path).toBe(false);
  }
});

test("an Arbesk asset is the third level, below its collection", () => {
  const account = "arbesk://0x00000000000000000000000000000000000000ab";
  expect(isArbeskAsset(`${account}/1~S/a~B.gltf`)).toBe(true);
  expect(isArbeskAsset(`${account}/1~S/a~B.gltf/`)).toBe(true);
  for (const path of [account, `${account}/`, `${account}/1~S`, `${account}/1~S/`, "/Users/me/1~S/a~B.gltf", "gdrive://acct/1~S/a~B.gltf", "arbesk://"]) {
    expect(isArbeskAsset(path), path).toBe(false);
  }
});

test("an Arbesk asset's other file is the fourth level, below its asset", () => {
  const asset = "arbesk://0x00000000000000000000000000000000000000ab/1~S/a~B.gltf";
  for (const path of [`${asset}/stl~B.stl`, `${asset}/gcode-mk4@bafyG1~B.gcode`, `${asset}/gcode%40bafyG1~B.gcode`, `${asset}/stl~`, `${asset}/stl~B.stl/`, `${asset}/${"a".repeat(64)}~B`]) {
    expect(isArbeskFile(path), path).toBe(true);
    expect(placeOf(path), path).toBe("arbesk");
    expect(isArbeskAsset(path), path).toBe(false);
  }
  // Not an id (or a pin) the backend parses: the account's row answers.
  const malformed = ["GCODE~x", "primary~x", "primary@bafy1~x", "my_file~x", "stl@~x", "stl@ba-fy~x", "%E0~x", `${"a".repeat(65)}~x`, "stl~%E0.stl", "stl~50%.stl"];
  // Nor is a path whose other segments the backend refuses: a bad address, a
  // token that isn't a number, an empty segment, a badly encoded name.
  const account = "arbesk://0x00000000000000000000000000000000000000ab";
  const elsewhere = ["arbesk://0xab/1~S/a~B.gltf/stl~x", `${account}/abc~S/a~B.gltf/stl~x`, `${account}//1~S/a~B.gltf/stl~x`, `${account}/1~S//a~B.gltf/stl~x`, `${account}/1~S%/a~B.gltf/stl~x`, `${account}/1~S/~B.gltf/stl~x`];
  for (const path of [asset, `${asset}/`, `${asset}/x`, `${asset}/~B.stl`, `${asset}/stl~B.stl/more~x`, ...malformed.map((file) => `${asset}/${file}`), ...elsewhere, "/Users/me/1~S/a~B.gltf/stl~B.stl", "gdrive://acct/1~S/a~B/stl~B.stl", "arbesk://"]) {
    expect(isArbeskFile(path), path).toBe(false);
  }
});

test("an Arbesk asset's other file's pin is the CID after its id's @", () => {
  const asset = "arbesk://0x00000000000000000000000000000000000000ab/1~S/a~B.gltf";
  expect(arbeskFilePin(`${asset}/gcode-mk4@bafyG1~B.gcode`)).toBe("bafyG1");
  expect(arbeskFilePin(`${asset}/gcode%40bafyG1~B.gcode/`)).toBe("bafyG1");
  for (const path of [`${asset}/stl~B.stl`, asset, `${asset}/stl@~x`, "/Users/me/1~S/a~B.gltf/stl@bafy1~B.stl"]) {
    expect(arbeskFilePin(path), path).toBeNull();
  }
});

test("an Arbesk collection is the second level, below its account", () => {
  const account = "arbesk://0x00000000000000000000000000000000000000ab";
  expect(isArbeskCollection(`${account}/1~S`)).toBe(true);
  expect(isArbeskCollection(`${account}/1~S/`)).toBe(true);
  for (const path of [account, `${account}/`, `${account}/1~S/a~B.gltf`, "/Users/me/1~S", "gdrive://acct/1~S", "arbesk://"]) {
    expect(isArbeskCollection(path), path).toBe(false);
  }
});

test("pathSegments and localSegments break a local path into its folders", () => {
  expect(pathSegments("/Users/me/")).toEqual({
    root: { kind: "local", key: "", path: "/", name: "" },
    segments: [
      { name: "Users", path: "/Users" },
      { name: "me", path: "/Users/me" },
    ],
  });
  expect(pathSegments("/").segments).toEqual([]);
  expect(localSegments("/Users/me", "/Users/me/a%20b//c")).toEqual([
    { name: "a%20b", path: "/Users/me/a%20b" },
    { name: "c", path: "/Users/me/a%20b/c" },
  ]);
  expect(localSegments("/Users/me", "/Users/me")).toEqual([]);
});

// prettier-ignore
test.each([
  ["/Users/me",              "/Users/me",      true],
  ["/Users/me/a/b",          "/Users/me",      true],
  ["/Users/me/a",            "/Users/me/",     true],
  ["/Users/meg",             "/Users/me",      false],
  ["/Users",                 "/Users/me",      false],
  ["/anything",              "/",              true],
  ["share://r1/Photos",      "share://r1",     true],
  ["sftp://me@host/docs",    "sftp://me@host/", true],
  ["sftp://me@host",         "sftp://me@host/", false],
])("isUnder(%j, %j) is %j", (path, dir, under) => {
  expect(isUnder(path, dir)).toBe(under);
});

// The containment Home, the Trash, volumes and dragged items use: "/" (Home
// when HOME=/) and a folder written with a trailing slash hold only themselves.
// prettier-ignore
test.each([
  ["/Users/me",              "/Users/me",      true],
  ["/Users/me/a/b",          "/Users/me",      true],
  ["/Users/meg",             "/Users/me",      false],
  ["/Users",                 "/Users/me",      false],
  ["/",                      "/",              true],
  ["/etc",                   "/",              false],
  ["/Volumes/USB",           "/",              false],
  ["/Users/me/",             "/Users/me/",     true],
  ["/Users/me/Documents",    "/Users/me/",     false],
  ["/Users/me/.Trash/a",     "/Users/me/.Trash", true],
])("isWithin(%j, %j) is %j", (path, dir, within) => {
  expect(isWithin(path, dir)).toBe(within);
});

// Why both stay (#149): for a folder written without a trailing slash they
// are the same test, so normalising Home would remove that difference, but
// Home is "/" when HOME=/. There the Home crumb and the Location column need
// "/" to hold only itself (isWithin), while a transfer into "/" must reload
// a folder being filled below it (isUnder, showLanded).
test("isUnder and isWithin differ only for a dir that is \"/\" or ends in a slash", () => {
  const paths = ["/", "/etc", "/Users", "/Users/me", "/Users/me/", "/Users/me/a", "/Users/meg", "/Users/me//a"];
  for (const dir of ["/Users", "/Users/me", "/Users/me/.Trash", "/Volumes/USB"]) {
    for (const path of paths) expect([path, dir, isUnder(path, dir)]).toEqual([path, dir, isWithin(path, dir)]);
  }
  expect([isUnder("/etc", "/"), isWithin("/etc", "/")]).toEqual([true, false]);
  expect([isUnder("/Users/me/a", "/Users/me/"), isWithin("/Users/me/a", "/Users/me/")]).toEqual([true, false]);
});

// Tauri's homeDir keeps $HOME as written; the app trims it once at start-up (#170).
// prettier-ignore
test.each([
  ["/Users/me",    "/Users/me"],
  ["/Users/me/",   "/Users/me"],
  ["/Users/me//",  "/Users/me"],
  ["/",            "/"],
  ["//",           "/"],
  ["",             ""],
])("homeFolder(%j) is %j", (home, folder) => {
  expect(homeFolder(home)).toBe(folder);
});

// prettier-ignore
test.each([
  ["/Users/me",                  true],
  ["pseudo:recent",              true],
  ["pseudo:starred",             true],
  ["pseudo:network",             true],
  // Only the pseudo locations the app has: an unknown one would break the path bar.
  ["pseudo:bogus",               false],
  ["pseudo:",                    false],
  [DRIVE_ROOT,                   true],
  ["share://r1/Photos",          true],
  ["sftp://me@host/",            true],
  ["sftp://me@host/docs",        true],
  ["Documents",                  false],
  ["https://example.com",        false],
  ["",                           false],
])("isKnownPath(%j) is %j", (path, known) => {
  expect(isKnownPath(path)).toBe(known);
});

test("segment helpers decode without throwing and find a scheme's key", () => {
  expect(decodeSegment("a%20b")).toBe("a b");
  expect(decodeSegment("%E0%A4%A")).toBe("%E0%A4%A");
  expect(cloudSegmentName("f1~a%20b")).toBe("a b");
  expect(cloudSegmentName("root")).toBe("root");
  expect(shareId("share://r1/Photos")).toBe("r1");
  expect(shareId("share://")).toBe("");
  expect(isCloudRoot("gdrive://acct")).toBe(true);
  expect(isCloudRoot(DRIVE_ROOT)).toBe(false);
  expect(isCloudRoot("/Users/me")).toBe(false);
});

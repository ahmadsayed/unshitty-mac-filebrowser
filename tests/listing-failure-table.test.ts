// listingReaction on its own: one case per row of the table. What the app
// shows for them is in listing-failure.test.ts.
import { expect, test } from "vitest";
import { listingReaction, type ListingContext, type ListingPhase } from "../src/listing-failure";
import { cleanError } from "../src/ui";

const NOT_PERMITTED = "Failed to read directory: Operation not permitted (os error 1)";
const MISSING = "Failed to read directory: No such file or directory (os error 2)";
const DENIED = "Failed to read directory: Permission denied (os error 13)";
const SHARE = "share://ticket/Photos";
const SFTP = "sftp://me@host/home";

const ctx = (phase: ListingPhase, more: Partial<ListingContext> = {}): ListingContext => ({
  phase,
  isMac: true,
  nameOf: (loc) => `name of ${loc}`,
  parentOf: (loc) => `parent of ${loc}`,
  ...more,
});

test("cleanError keeps the part people read", () => {
  expect(cleanError(DENIED)).toBe("Permission denied");
  expect(cleanError("Connect to this server first.")).toBe("Connect to this server first.");
  expect(cleanError(new Error("boom"))).toBe("boom");
});

const rows: [string, string, unknown, ListingContext, object][] = [
  // A received share
  ["reload of a share", SHARE, "gone", ctx("reload", { share: "no-access" }), { kind: "sharePage", status: "offline" }],
  ["going to a share without access", SHARE, "gone", ctx("navigate", { share: "no-access" }), { kind: "sharePage", status: "no-access" }],
  ["going to a share that is online", SHARE, "gone", ctx("navigate", { share: "online" }), { kind: "sharePage", status: "offline" }],
  ["a first view of a share", SHARE, "gone", ctx("first"), { kind: "sharePage", status: "offline" }],
  ["a typed share path", SHARE, "gone", ctx("entry"), { kind: "toast", text: `Unable to find “${SHARE}”: gone`, grantAccess: false }],
  // The path entry
  ["a typed path on a server that isn't connected", SFTP, "Connect to this server first.", ctx("entry"), { kind: "connect", url: SFTP }],
  ["a typed path a privacy block refuses", "/System", NOT_PERMITTED, ctx("entry"), {
    kind: "toast", text: "Unable to access “/System”: Operation not permitted", grantAccess: true,
  }],
  ["the same on Linux", "/System", NOT_PERMITTED, ctx("entry", { isMac: false }), {
    kind: "toast", text: "Unable to access “/System”: Operation not permitted", grantAccess: false,
  }],
  ["a typed path that doesn't exist", "/nope", MISSING, ctx("entry"), {
    kind: "toast", text: "Unable to find “/nope”: No such file or directory", grantAccess: false,
  }],
  // Going somewhere
  ["going to a folder a privacy block refuses", "/System", NOT_PERMITTED, ctx("navigate"), {
    kind: "toast", text: "Unable to access “name of /System”: Operation not permitted", grantAccess: true,
  }],
  ["going to a folder that went away", "/a/gone", MISSING, ctx("navigate"), {
    kind: "toast", text: "Unable to access “name of /a/gone”: No such file or directory", grantAccess: false,
  }],
  ["going to a server that isn't connected", SFTP, "Connect to this server first.", ctx("navigate"), {
    kind: "toast", text: "Unable to access “name of sftp://me@host/home”: Connect to this server first.", grantAccess: false,
  }],
  // The first view
  ["a first view a privacy block refuses", "/System", NOT_PERMITTED, ctx("first"), { kind: "errorPage" }],
  // A reload
  ["reload of a local folder that went away", "/a/gone", MISSING, ctx("reload"), { kind: "goTo", parent: "parent of /a/gone" }],
  ["reload of a server folder that went away", `${SFTP}/gone`, MISSING, ctx("reload"), { kind: "errorPage" }],
  ["reload of a folder that can't be read", "/a/locked", DENIED, ctx("reload"), { kind: "errorPage" }],
];

test.each(rows)("%s", (_label, loc, err, context, expected) => {
  expect(listingReaction(loc, err, context)).toEqual({ ...expected, message: cleanError(err) });
});

test("a name is only asked for when a toast shows it", () => {
  const throws = () => {
    throw new URIError("URI malformed");
  };
  const typed = "sftp://us%zz@host";
  expect(listingReaction(typed, "Connect to this server first.", ctx("entry", { nameOf: throws, parentOf: throws })))
    .toMatchObject({ kind: "connect", url: typed });
  expect(listingReaction("/a/locked", DENIED, ctx("reload", { nameOf: throws, parentOf: throws }))).toMatchObject({ kind: "errorPage" });
});

test("a Drive listing that lost its credential says which one is needed", () => {
  const drive = "gdrive://acct/root~My%20Drive";
  const expired = "Google Drive session expired: Server returned error response: invalid_grant: Token has been expired or revoked.";
  expect(listingReaction(drive, expired, ctx("navigate"))).toMatchObject({
    kind: "toast",
    driveAuth: "sign-in",
  });
  expect(listingReaction(drive, expired, ctx("reload"))).toMatchObject({
    kind: "errorPage",
    driveAuth: "sign-in",
  });
  expect(
    listingReaction(drive, "Google Drive needs your own OAuth client. Add one in the Connect dialog.", ctx("navigate"))
  ).toMatchObject({ kind: "toast", driveAuth: "client" });
});

test("only Drive paths ask for a Drive credential", () => {
  const expired = "Google Drive sign-in expired. Sign in again.";
  expect(listingReaction("/a/folder", expired, ctx("navigate"))).not.toHaveProperty("driveAuth");
  expect(listingReaction(SHARE, expired, ctx("navigate"))).not.toHaveProperty("driveAuth");
  expect(listingReaction("sftp://me@host/x", expired, ctx("navigate"))).not.toHaveProperty("driveAuth");
});

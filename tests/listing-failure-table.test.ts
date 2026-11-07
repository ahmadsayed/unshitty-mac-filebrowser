// listingReaction on its own: one case per row of the table. What the app
// shows for them is in listing-failure.test.ts.
import { expect, test } from "vitest";
import { cleanError, listingReaction, type ListingContext, type ListingPhase } from "../src/listing-failure";

const NOT_PERMITTED = "Failed to read directory: Operation not permitted (os error 1)";
const MISSING = "Failed to read directory: No such file or directory (os error 2)";
const DENIED = "Failed to read directory: Permission denied (os error 13)";
const SHARE = "share://ticket/Photos";
const SFTP = "sftp://me@host/home";

const ctx = (phase: ListingPhase, more: Partial<ListingContext> = {}): ListingContext => ({
  phase,
  isMac: true,
  name: "Name",
  parent: "/parent",
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
  ["going to a folder a privacy block refuses", "/System", NOT_PERMITTED, ctx("navigate", { name: "System" }), {
    kind: "toast", text: "Unable to access “System”: Operation not permitted", grantAccess: true,
  }],
  ["going to a folder that went away", "/a/gone", MISSING, ctx("navigate", { name: "gone" }), {
    kind: "toast", text: "Unable to access “gone”: No such file or directory", grantAccess: false,
  }],
  ["going to a server that isn't connected", SFTP, "Connect to this server first.", ctx("navigate", { name: "home" }), {
    kind: "toast", text: "Unable to access “home”: Connect to this server first.", grantAccess: false,
  }],
  // The first view
  ["a first view a privacy block refuses", "/System", NOT_PERMITTED, ctx("first"), { kind: "errorPage" }],
  // A reload
  ["reload of a local folder that went away", "/a/gone", MISSING, ctx("reload"), { kind: "goTo", parent: "/parent" }],
  ["reload of a server folder that went away", `${SFTP}/gone`, MISSING, ctx("reload"), { kind: "errorPage" }],
  ["reload of a folder that can't be read", "/a/locked", DENIED, ctx("reload"), { kind: "errorPage" }],
];

test.each(rows)("%s", (_label, loc, err, context, expected) => {
  expect(listingReaction(loc, err, context)).toEqual({ ...expected, message: cleanError(err) });
});

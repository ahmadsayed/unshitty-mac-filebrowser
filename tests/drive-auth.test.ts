// Which Google Drive failures ask for a credential: a missing or rejected
// OAuth client, or a sign-in that expired or was revoked. The messages are
// the ones src-tauri/src/cloud sends.
import { expect, test } from "vitest";
import { driveAuthNeed } from "../src/drive-auth";

test("a missing client asks for the client JSON", () => {
  expect(
    driveAuthNeed("Google Drive needs your own OAuth client. Add one in the Connect dialog.")
  ).toBe("client");
  expect(
    driveAuthNeed(
      "Google Drive session expired. Disconnect Google Drive, then connect again with your own OAuth client."
    )
  ).toBe("client");
});

test("a rejected client asks for the client too", () => {
  for (const message of [
    "Google Drive session expired: Server returned error response: invalid_client: The OAuth client was not found.",
    "Google Drive session expired: Server returned error response: invalid_client.",
  ]) {
    expect(driveAuthNeed(message), message).toBe("client");
  }
});

test("an expired or revoked sign-in asks to sign in again", () => {
  for (const message of [
    "Google Drive sign-in expired. Sign in again.",
    "Google Drive session expired: Server returned error response: invalid_grant: Token has been expired or revoked.",
    "Google Drive is not connected. Sign in again.",
    "Google Drive is not connected.",
  ]) {
    expect(driveAuthNeed(message), message).toBe("sign-in");
  }
});

test("other Drive failures are left to their own messages", () => {
  for (const message of [
    "Google Drive refused the request: The user does not have sufficient permissions.",
    "Google Drive is temporarily unavailable. Try again.",
    "The Google Drive item no longer exists.",
    "Google Drive is connected read-only. Disconnect and connect again to allow changes.",
    "Failed to read directory: Operation not permitted",
    "Network is unreachable",
  ]) {
    expect(driveAuthNeed(message), message).toBeNull();
  }
});

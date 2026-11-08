// What a failed Google Drive request says about the connection, so the view
// can offer the step that fixes it instead of only a toast: the user's own
// OAuth client is missing or no longer accepted ("client"), or the sign-in
// expired or was revoked ("sign-in"). Messages are the backend's
// (src-tauri/src/cloud/drive.rs, oauth.rs and service.rs), matched here as
// they are tested in tests/drive-auth.test.ts.

export type DriveAuthNeed = "client" | "sign-in";

/** The client must be added or replaced before signing in again. */
const CLIENT =
  /own OAuth client|invalid_client|OAuth client was not found|needs your own OAuth client/i;

/** The stored token no longer works; a new sign-in is needed. */
const SIGN_IN =
  /sign-in expired|session expired|not connected|invalid_grant|expired or revoked|sign in again/i;

/** Which credential Google Drive needs, or null when the failure isn't about one. */
export function driveAuthNeed(message: string): DriveAuthNeed | null {
  // A message that names the client asks for a client even when it also says
  // the session expired ("…with your own OAuth client").
  if (CLIENT.test(message)) return "client";
  if (SIGN_IN.test(message)) return "sign-in";
  return null;
}

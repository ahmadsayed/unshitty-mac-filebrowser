// A Google Drive listing whose sign-in expired, or that has no OAuth client,
// opens the dialog that fixes it (and the guide) instead of only a toast.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, dialogHeading, goTo, names, respond, settle, toasts } from "./app";

const DRIVE = "gdrive://mock/mock-root~My%20Drive";
const EXPIRED =
  "Google Drive session expired: Server returned error response: invalid_grant: Token has been expired or revoked.";
const NO_CLIENT = "Google Drive needs your own OAuth client. Add one in the Connect dialog.";

type MockCalls = { failNext(cmd: string, error: string): void; reset(): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;

const CLIENT = JSON.stringify({
  installed: { client_id: "123-abc.apps.googleusercontent.com", client_secret: "GOCSPX-test" },
});

const buttonLabels = () => $$(".dialog-responses .btn").map((button) => button.textContent);
const linkLabels = () => $$(".dialog-links button").map((button) => button.textContent);

beforeAll(async () => {
  await boot();
});

test("with no OAuth client, a failed Drive listing opens the client setup with the guide", async () => {
  calls().failNext("list_dir", NO_CLIENT);
  await goTo(DRIVE);
  await settle(20);
  expect(dialogHeading()).toBe("Add Your Google Client");
  expect(linkLabels()).toEqual(["Open Guide", "Choose File…"]);
  expect($(".dialog-heading").textContent).toBe("Add Your Google Client");

  // Adding the client moves on to signing in, as the Connect button does.
  $<HTMLTextAreaElement>(".client-json").value = CLIENT;
  await respond("Add Client");
  await settle(15);
  expect(dialogHeading()).toBe("Connect to Google Drive");
});

test("an expired sign-in opens Sign In to Google Drive and signs the same account back in", async () => {
  // Sign in from the Connect dialog the previous test opened.
  await respond("Open Browser");
  await settle(20);
  expect(names()).toContain("hello.txt");

  // Network lists the account; opening Drive fails as an expired token would.
  calls().failNext("list_dir", EXPIRED);
  await goTo(DRIVE);
  await settle(20);
  expect(dialogHeading()).toBe("Sign In to Google Drive");
  expect($(".dialog-layer").textContent).toContain("me@example.com");
  expect($(".dialog-layer").textContent).toContain("expired or was revoked");
  expect(linkLabels()).toEqual(["Open Guide", "Change Client…"]);
  expect(buttonLabels()).toEqual(["Cancel", "Sign In"]);

  await respond("Sign In");
  await settle(20);
  expect(document.querySelector(".dialog-layer")).toBeNull();
  expect(toasts().join(" ")).toContain("Signed in to me@example.com");
  // The folder that failed shows once the account is back.
  expect(names()).toContain("hello.txt");
  expect((await invoke<Array<{ id: string }>>("drive_accounts")).length).toBe(1);
});

test("a rejected OAuth client says so and offers Change Client…", async () => {
  calls().failNext(
    "list_dir",
    "Google Drive session expired: Server returned error response: invalid_client: The OAuth client was not found."
  );
  await goTo(DRIVE);
  await settle(20);
  expect(dialogHeading()).toBe("Sign In to Google Drive");
  expect($(".dialog-layer").textContent).toContain("no longer accepts the OAuth client");
  expect(linkLabels()).toContain("Change Client…");
  await respond("Cancel");
});

test("Change Client… from the expired dialog disconnects first, as the backend requires", async () => {
  calls().failNext("list_dir", EXPIRED);
  await goTo(DRIVE);
  await settle(20);
  expect(dialogHeading()).toBe("Sign In to Google Drive");

  await click($$(".dialog-links button").find((button) => button.textContent === "Change Client…")!);
  await settle(20);
  expect(dialogHeading()).toBe("Add Your Google Client");
  expect(await invoke<unknown[]>("drive_accounts")).toEqual([]);
  await respond("Cancel");
});

test("with a client but no account, a Drive failure offers Connect, not client setup", async () => {
  await invoke("drive_client_set", { json: CLIENT });
  calls().failNext("list_dir", "Google Drive is not connected. Sign in again.");
  await goTo(DRIVE);
  await settle(20);
  expect(dialogHeading()).toBe("Connect to Google Drive");
  expect(buttonLabels()).toEqual(["Cancel", "Open Browser"]);
  await respond("Cancel");
});

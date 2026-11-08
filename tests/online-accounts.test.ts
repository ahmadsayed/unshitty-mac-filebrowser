// Online Accounts: the dialog that connects Google Drive (and later other
// providers), each account's dialog, several Google accounts in Network,
// "Sign in again" after a failed request, and Change Client… asking before it
// disconnects accounts that still work.
import { beforeAll, expect, test } from "vitest";
import { invoke, setMockDriveEmail } from "../dev/mock-tauri/core";
import {
  $, $$, accountGroups, boot, chooseAccountRow, click, closeOverlays, dialogHeading, goTo, key, names, openOnlineAccounts,
  respond, selectedNames, settle, toasts,
} from "./app";

const CLIENT = JSON.stringify({
  installed: { client_id: "123-abc.apps.googleusercontent.com", client_secret: "GOCSPX-test" },
});
const EXPIRED =
  "Google Drive session expired: Server returned error response: invalid_grant: Token has been expired or revoked.";
const SECOND = "gdrive://mock2/mock-root~My%20Drive";

type MockCalls = { failNext(cmd: string, error: string): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
const buttons = () => $$(".dialog-layer .dialog-responses .btn").map((button) => button.textContent);
const links = () => $$(".dialog-layer .dialog-links button").map((button) => button.textContent);

async function openNetwork(): Promise<void> {
  await click($$(".sidebar-row").find((row) => row.textContent?.trim() === "Network")!);
  await settle(20);
}

/** Connect a Google account through Online Accounts, signing in as `email`. */
async function connect(email: string): Promise<void> {
  setMockDriveEmail(email);
  await openOnlineAccounts();
  await chooseAccountRow("Connect an Account", "Google Drive");
  expect(dialogHeading()).toBe("Connect to Google Drive");
  await respond("Open Browser");
  await settle(20);
}

beforeAll(async () => {
  await boot();
  await invoke("drive_client_set", { json: CLIENT });
});

test("Online Accounts opens from the primary menu with the first row focused", async () => {
  await openOnlineAccounts();
  expect(dialogHeading()).toBe("Online Accounts");
  expect($(".dialog-layer .dialog-text").textContent).toBe("Connect an account to browse its files in Network.");
  expect(accountGroups()).toEqual([{ title: "Connect an Account", rows: ["Google Drive"] }]);
  expect(document.activeElement?.classList.contains("action-row")).toBe(true);
  expect(buttons()).toEqual(["Close"]);
  await key("Escape", { target: document.activeElement ?? document.body });
  expect(document.querySelector(".dialog-layer")).toBeNull();
});

test("a connected account lists under Your Accounts, and a second Google account adds a second row", async () => {
  await connect("me@example.com");
  expect(toasts().join(" ")).toContain("Connected to me@example.com");
  expect($(".crumb.current")?.textContent).toContain("My Drive");

  await connect("work@example.com");
  expect(toasts().join(" ")).toContain("Connected to work@example.com");

  await openOnlineAccounts();
  expect(accountGroups()).toEqual([
    { title: "Your Accounts", rows: ["Google Drive · me@example.com", "Google Drive · work@example.com"] },
    { title: "Connect an Account", rows: ["Google Drive"] },
  ]);
  await closeOverlays();

  // Network lists both, each with its address.
  await openNetwork();
  expect(names().filter((name) => name === "Google Drive")).toHaveLength(2);
  expect($("#view").textContent).toContain("me@example.com");
  expect($("#view").textContent).toContain("work@example.com");
});

test("signing in to an account that is already connected keeps one row", async () => {
  await connect("me@example.com");
  await openOnlineAccounts();
  expect(accountGroups()[0].rows).toHaveLength(2);
  await closeOverlays();
});

test("Up from the second account's My Drive goes to Network and selects it", async () => {
  await goTo(SECOND);
  expect(names()).toContain("hello.txt");
  await key("ArrowUp", { alt: true });
  await settle(20);
  expect($(".crumb.current")?.textContent).toContain("Network");
  expect(selectedNames()).toEqual(["Google Drive"]);
  expect($(".item.selected").textContent).toContain("work@example.com");
});

test("an account's dialog says it is connected, opens its files and offers Disconnect…", async () => {
  await openOnlineAccounts();
  await chooseAccountRow("Your Accounts", "Google Drive");
  expect(dialogHeading()).toBe("Google Drive");
  expect($(".dialog-layer").textContent).toContain("me@example.com");
  expect($(".dialog-layer .account-status").textContent).toBe("Connected");
  expect(links()).toEqual(["Browse Files", "Change Client…"]);
  expect(buttons()).toEqual(["Close", "Disconnect…"]);

  await click($$(".dialog-layer .dialog-links button").find((button) => button.textContent === "Browse Files")!);
  await settle(20);
  expect(document.querySelector(".dialog-layer")).toBeNull();
  expect(names()).toContain("hello.txt");
  expect($(".crumb.current")?.textContent).toContain("My Drive");
});

test("a request that needs a sign-in marks the account until it signs in again", async () => {
  calls().failNext("list_dir", EXPIRED);
  await goTo(SECOND);
  await settle(20);
  expect(dialogHeading()).toBe("Sign In to Google Drive");
  await respond("Cancel");

  await openOnlineAccounts();
  expect(accountGroups()[0].rows).toEqual([
    "Google Drive · me@example.com",
    "Google Drive · work@example.com · Sign in again",
  ]);
  await chooseAccountRow("Your Accounts", "Google Drivework@example.com");
  expect($(".dialog-layer .account-status").textContent).toContain("expired or was revoked");
  expect(buttons()).toEqual(["Close", "Disconnect…", "Sign In Again"]);

  await respond("Sign In Again");
  await settle(10);
  expect(dialogHeading()).toBe("Sign In to Google Drive");
  expect($(".dialog-layer").textContent).toContain("work@example.com");
  // In the browser, the user signs in as the work account again.
  setMockDriveEmail("work@example.com");
  await respond("Sign In");
  await settle(20);
  expect(toasts().join(" ")).toContain("Signed in to work@example.com");

  await openOnlineAccounts();
  expect(accountGroups()[0].rows).toEqual(["Google Drive · me@example.com", "Google Drive · work@example.com"]);
  await closeOverlays();
});

test("Change Client… asks before disconnecting accounts that still work, and Cancel keeps them", async () => {
  await openOnlineAccounts();
  await chooseAccountRow("Your Accounts", "Google Driveme@example.com");
  await click($$(".dialog-layer .dialog-links button").find((button) => button.textContent === "Change Client…")!);
  await settle(15);
  expect(dialogHeading()).toBe("Change Google Client?");
  expect($$(".dialog-layer").at(-1)!.textContent).toContain("2 Google accounts are disconnected first");
  expect(buttons()).toEqual(["Cancel", "Disconnect and Change"]);
  await respond("Cancel");
  expect(document.querySelector(".dialog-layer")).toBeNull();
  expect(await invoke<unknown[]>("drive_accounts")).toHaveLength(2);
});

test("Disconnect… from an account's dialog removes only that account", async () => {
  await goTo(SECOND);
  await openOnlineAccounts();
  await chooseAccountRow("Your Accounts", "Google Drivework@example.com");
  await respond("Disconnect…");
  await settle(10);
  expect(dialogHeading()).toBe("Disconnect Google Drive?");
  expect($(".dialog-layer").textContent).toContain("work@example.com");
  await respond("Disconnect");
  await settle(20);
  expect(toasts().join(" ")).toContain("Disconnected from Google Drive");
  // It was showing the disconnected account's files: Network instead.
  expect($(".crumb.current")?.textContent).toContain("Network");
  expect(names().filter((name) => name === "Google Drive")).toHaveLength(1);
  expect((await invoke<Array<{ email: string }>>("drive_accounts")).map((a) => a.email)).toEqual(["me@example.com"]);
});

test("Change Client… with the agreement disconnects every Google account and opens the client setup", async () => {
  await openOnlineAccounts();
  await chooseAccountRow("Your Accounts", "Google Drive");
  await click($$(".dialog-layer .dialog-links button").find((button) => button.textContent === "Change Client…")!);
  await settle(15);
  expect($$(".dialog-layer").at(-1)!.textContent).toContain("“me@example.com” is disconnected first");
  await respond("Disconnect and Change");
  await settle(20);
  expect(dialogHeading()).toBe("Add Your Google Client");
  expect(await invoke<unknown[]>("drive_accounts")).toEqual([]);
  expect((await invoke<{ configured: boolean }>("drive_client_status")).configured).toBe(false);
  await respond("Cancel");
  await openOnlineAccounts();
  expect(accountGroups()).toEqual([{ title: "Connect an Account", rows: ["Google Drive"] }]);
  await closeOverlays();
});

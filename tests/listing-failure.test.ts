// What a folder that can't be listed turns into, depending on how the view
// got there: the first view shows an error page, going to it keeps the view
// and shows a toast (with Grant Access for a macOS privacy block), the path
// entry checks before going, and a share shows its status page, whether
// Network already knew its sharer was offline or the listing found out.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, click, closeOverlays, dialogHeading, goTo, key, names, open, settle, title } from "./app";

const HOME = "/Users/me";
const place = (label: string) => $$(".sidebar-row").find((row) => row.textContent?.trim() === label)!;
const share = () =>
  (globalThis as unknown as {
    __mockShare: { reset(): void; setOnline(v: boolean): void; setNoAccess(v: boolean): void; setUnreachable(v: boolean): void };
  }).__mockShare;

type MockCalls = { hold(cmd: string): void; release(cmd: string): void; failNext(cmd: string, error: string): void; reset(): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;

/** The toast that says `text`, if one is on screen. */
const toastWith = (text: string) => $$(".toast").find((t) => t.textContent?.includes(text));

beforeAll(async () => {
  // The window opens on a folder macOS won't let it read.
  history.replaceState(null, "", "/?path=/System");
  await boot();
  history.replaceState(null, "", "/");
});

test("the first view of a folder that can't be read is an error page", () => {
  expect(title()).toBe("System");
  expect($("#view").textContent).toContain("Unable to Access Location");
  expect($("#view").textContent).toContain("Operation not permitted");
});

test("reloading the error page keeps it", async () => {
  await key("F5");
  await settle(20);
  expect(title()).toBe("System");
  expect($("#view").textContent).toContain("Unable to Access Location");
});

test("going back to it keeps the view and offers Grant Access", async () => {
  await goTo(HOME);
  expect(title()).toBe("Home");
  await key("Backspace");
  await settle(20);
  expect(title()).toBe("Home");
  expect(names()).toContain("Documents");
  const toast = toastWith("Unable to access “System”: Operation not permitted");
  expect(toast).toBeTruthy();
  expect(toast!.querySelector(".btn")?.textContent).toBe("Grant Access");
});

test("the path entry names the typed path, with Grant Access only for a privacy block", async () => {
  await goTo("/System");
  expect(title()).toBe("Home");
  const blocked = toastWith("Unable to access “/System”: Operation not permitted");
  expect(blocked?.querySelector(".btn")?.textContent).toBe("Grant Access");
  await closeOverlays();

  await goTo(`${HOME}/Nowhere`);
  expect(title()).toBe("Home");
  const missing = toastWith(`Unable to find “${HOME}/Nowhere”: No such file or directory`);
  expect(missing).toBeTruthy();
  expect(missing!.querySelector(".btn")).toBeNull();
  await closeOverlays();
});

test("the path entry asks to connect to an SSH server that isn't connected", async () => {
  (globalThis as unknown as { __mockSftp: { reset(): void } }).__mockSftp.reset();
  await goTo("sftp://user@host/home");
  expect(dialogHeading()).toBe("Connect to Server");
  expect($<HTMLInputElement>("#sftp-server").value).toBe("sftp://user@host/home");
  await closeOverlays();
  expect(title()).toBe("Home");
});

test("a bare name typed in Recent, Starred or Network is refused, not read from the app's working directory", async () => {
  // resolveTypedPath leaves it as typed there (tests/paths.test.ts); the
  // backend's place::local refuses a relative path.
  for (const label of ["Recent", "Starred", "Network"]) {
    await click(place(label));
    await settle(20);
    expect(title()).toBe(label);
    await goTo("Documents");
    expect(title()).toBe(label);
    const refused = toastWith("Unable to find “Documents”: “Documents” is not a local file.");
    expect(refused, label).toBeTruthy();
    expect(refused!.querySelector(".btn")).toBeNull();
    await closeOverlays();
  }
  await goTo(HOME);
  expect(title()).toBe("Home");
});

test("reloading a share whose sharer went offline, or took access away, shows its status page", async () => {
  share().reset();
  await invoke("add_received", { link: `share://${"mockreload".padEnd(64, "a")}` });
  await click(place("Network"));
  await settle(20);
  await open("Photos");
  await settle(20);
  expect(names()).toEqual(["Photos", "hello.txt"]);

  share().setOnline(false);
  await key("F5");
  await settle(20);
  expect($("#view").textContent).toContain("Ahmed Is Offline");

  share().setOnline(true);
  await key("F5");
  await settle(20);
  expect(names()).toEqual(["Photos", "hello.txt"]);

  share().setNoAccess(true);
  await key("F5");
  await settle(20);
  expect($("#view").textContent).toContain("No Access");
  expect($("#view").textContent).toContain("Ahmed stopped sharing this folder with you.");
  share().reset();
});

// Network still says the share is online, so these go past the status check
// and reach the listing, which fails: listingReaction picks the status page.
test("opening a share whose sharer dropped since Network asked shows its status page", async () => {
  share().reset();
  await invoke("add_received", { link: `share://${"mockdropped".padEnd(64, "a")}` });
  await click(place("Network"));
  await settle(20);
  share().setUnreachable(true);
  await open("Photos");
  await settle(20);
  expect(title()).toBe("Photos");
  expect($("#view").textContent).toContain("Ahmed Is Offline");
  share().reset();
});

test("reloading a share whose sharer dropped since Network asked shows its status page", async () => {
  share().reset();
  await invoke("add_received", { link: `share://${"mockdropped".padEnd(64, "a")}` });
  await click(place("Network"));
  await settle(20);
  await open("Photos");
  await settle(20);
  expect(names()).toEqual(["Photos", "hello.txt"]);

  share().setUnreachable(true);
  await key("F5");
  await settle(20);
  expect($("#view").textContent).toContain("Ahmed Is Offline");
  expect(names()).toEqual([]);

  share().setUnreachable(false);
  await key("F5");
  await settle(20);
  expect(names()).toEqual(["Photos", "hello.txt"]);
  share().reset();
});

// Opening a share lists it only when Network says it is online; if Network
// learns that access was taken away while that listing is on its way (here
// a reload of Network; a slow sharer can take a connect timeout to answer),
// the failed listing shows No Access rather than Offline. This is the one
// way listingReaction's no-access row is reached when going to a share.
test("a share whose sharer took access away while it opened shows No Access", async () => {
  share().reset();
  calls().reset();
  await invoke("add_received", { link: `share://${"mockrevoked".padEnd(64, "a")}` });
  await click(place("Network"));
  await settle(20);
  calls().hold("list_dir");
  calls().failNext("list_dir", "Ahmed stopped sharing this folder with you.");
  await open("Photos");
  share().setNoAccess(true);
  await key("F5");
  await settle(20);
  expect(title()).toBe("Network");
  calls().release("list_dir");
  await settle(20);
  expect(title()).toBe("Photos");
  expect($("#view").textContent).toContain("No Access");
  expect($("#view").textContent).toContain("Ahmed stopped sharing this folder with you.");
  share().reset();
  calls().reset();
});

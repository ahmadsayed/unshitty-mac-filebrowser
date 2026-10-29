// Check for Updates: the main menu item and every dialog of the flow, against
// the mock backend's __mockUpdate (see docs/superpowers/specs/2026-10-29-updates-design.md).
import { beforeAll, beforeEach, expect, test, vi } from "vitest";
import { endOperation, startOperation } from "../src/operations";
import { $, $$, boot, chooseMenu, click, closeOverlays, dialogHeading, key, menuLabels, respond, settle } from "./app";

type UpdateFailure = { kind: string; detail: string };
type MockUpdate = {
  set(o: { latest?: string | null; kind?: string; failCheck?: UpdateFailure; failInstall?: UpdateFailure; total?: number | null; lateCancel?: boolean }): void;
  hold(): void; step(bytes: number): void; release(): void; holdCheck(): void; releaseCheck(): void;
  installed(): number; restarts(): number; reset(): void;
};
const mock = () => (globalThis as unknown as { __mockUpdate: MockUpdate }).__mockUpdate;

beforeAll(async () => {
  await boot();
});

beforeEach(async () => {
  mock().reset();
  await closeOverlays();
});

async function check(): Promise<void> {
  await click($("#btn-main-menu"));
  await chooseMenu("Check for Updates…");
  await settle(20);
}

const dialogText = () => [...document.querySelectorAll(".dialog-layer")].at(-1)?.textContent ?? "";
const buttons = () => $$(".dialog-layer .dialog-responses button").map((b) => b.textContent?.trim());
const button = (label: string) => $$(".dialog-layer .dialog-responses button").find((b) => b.textContent?.trim() === label) as HTMLButtonElement;
const noDialog = () => document.querySelector(".dialog-layer") === null;

async function offerAndDownload(): Promise<void> {
  mock().set({ latest: "0.5.4" });
  mock().hold();
  await check();
  await respond("Download and Install");
}

test("the menu offers Check for Updates above About", async () => {
  await click($("#btn-main-menu"));
  const labels = menuLabels();
  expect(labels.indexOf("Check for Updates…")).toBe(labels.indexOf("About File Browser") - 1);
  await closeOverlays();
});

test("Cancel while checking ignores the late answer", async () => {
  mock().holdCheck();
  await check();
  expect(dialogText()).toContain("Checking for updates…");
  await respond("Cancel");
  mock().releaseCheck();
  await settle(20);
  expect(noDialog()).toBe(true);
});

test("up to date", async () => {
  await check();
  expect(dialogText()).toContain("File Browser 0.5.3 is the latest version.");
  expect(buttons()).toEqual(["Close"]);
});

test("offline, then Try Again", async () => {
  mock().set({ failCheck: { kind: "offline", detail: "error sending request" } });
  await check();
  expect(dialogText()).toContain("Could not check for updates.");
  expect(dialogText()).toContain("No internet connection.");
  expect(buttons()).toEqual(["Close", "Try Again"]);
  mock().set({ failCheck: undefined, latest: "0.5.4" });
  await respond("Try Again");
  expect(dialogText()).toContain("File Browser 0.5.4 is available. You have 0.5.3.");
});

test("an offer on an AppImage defaults to Not Now", async () => {
  mock().set({ latest: "0.5.4" });
  await check();
  expect(dialogText()).toContain("File Browser 0.5.4 is available. You have 0.5.3.");
  expect($$(".dialog-layer .dialog-links button").map((b) => b.textContent)).toEqual(["What's New"]);
  expect(buttons()).toEqual(["Not Now", "Download and Install"]);
  await key("Enter");
  await settle(20);
  expect(noDialog()).toBe(true);
  expect(mock().installed()).toBe(0);
});

test("an offer on a package install points to the package manager", async () => {
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  mock().set({ latest: "0.5.4", kind: "linux_package" });
  await check();
  expect(dialogText()).toContain("Update it with your system's package manager.");
  expect(buttons()).toEqual(["Close", "Open Release Page"]);
  await respond("Open Release Page");
  expect(log).toHaveBeenCalledWith("mock openUrl", expect.stringMatching(/\/releases\/tag\/v0\.5\.4$/));
  log.mockRestore();
});

test("a dev build never offers to install", async () => {
  mock().set({ latest: "0.5.4", kind: "dev_build" });
  await check();
  expect(dialogText()).toContain("Updates are installed only in release builds.");
  expect(buttons()).not.toContain("Download and Install");
});

test("download, progress, install, restart", async () => {
  await offerAndDownload();
  mock().step(23_400_000);
  await settle(5);
  expect(dialogText()).toContain("Downloading… 23.4 MB of 91.4 MB");
  mock().release();
  await settle(20);
  expect(dialogHeading()).toBe("File Browser 0.5.4 is installed.");
  expect(dialogText()).toContain("Restart to use it.");
  expect(buttons()).toEqual(["Later", "Restart Now"]);
  await respond("Restart Now");
  expect(mock().restarts()).toBe(1);
});

test("progress without a total shows only what arrived", async () => {
  mock().set({ total: null });
  await offerAndDownload();
  mock().step(23_400_000);
  await settle(5);
  expect(dialogText()).toContain("Downloading… 23.4 MB");
  expect(dialogText()).not.toContain(" of ");
});

for (const how of ["Cancel", "Escape"] as const) {
  test(`${how} during the download installs nothing, and a new try works`, async () => {
    await offerAndDownload();
    if (how === "Cancel") await respond("Cancel");
    else await key("Escape");
    await settle(20);
    expect(noDialog()).toBe(true);
    mock().release();
    await settle(20);
    expect(mock().installed()).toBe(0);
    expect(noDialog()).toBe(true);
    await check();
    await respond("Download and Install");
    await settle(20);
    expect(mock().installed()).toBe(1);
  });
}

test("a Cancel that comes too late still shows the installed update", async () => {
  // The backend was already installing: Cancel can't stop it, and hiding the
  // result would leave the user unaware that the app was replaced.
  mock().set({ lateCancel: true });
  await offerAndDownload();
  await respond("Cancel");
  mock().release();
  await settle(20);
  expect(mock().installed()).toBe(1);
  expect(dialogHeading()).toBe("File Browser 0.5.4 is installed.");
});

test("Restart Now waits for operations, even ones started later", async () => {
  const noop = () => {};
  const first = startOperation("Copying “todo.txt”", { totalItems: 1, onCancel: noop });
  mock().set({ latest: "0.5.4" });
  await check();
  await respond("Download and Install");
  await settle(20);
  expect(button("Restart Now").disabled).toBe(true);
  expect(dialogText()).toContain("Waiting for operations to finish…");
  endOperation(first);
  await settle(5);
  expect(button("Restart Now").disabled).toBe(false);
  expect(dialogText()).not.toContain("Waiting for operations to finish…");
  const second = startOperation("Copying “Notes.md”", { totalItems: 1, onCancel: noop });
  await settle(5);
  expect(button("Restart Now").disabled).toBe(true);
  endOperation(second);
  await settle(5);
  expect(button("Restart Now").disabled).toBe(false);
});

test("Later keeps the session", async () => {
  mock().set({ latest: "0.5.4" });
  await check();
  await respond("Download and Install");
  await settle(20);
  await respond("Later");
  expect(noDialog()).toBe(true);
  expect(mock().restarts()).toBe(0);
});

test("a failed download offers Try Again", async () => {
  mock().set({ latest: "0.5.4", failInstall: { kind: "offline", detail: "Connection reset" } });
  await check();
  await respond("Download and Install");
  await settle(20);
  expect(dialogText()).toContain("The download failed.");
  expect(dialogText()).toContain("Connection reset");
  expect(buttons()).toEqual(["Close", "Try Again"]);
  mock().set({ failInstall: undefined });
  await respond("Try Again");
  await settle(20);
  expect(mock().installed()).toBe(1);
});

test("an unverified update is not installed", async () => {
  mock().set({ latest: "0.5.4", failInstall: { kind: "unverified", detail: "signature mismatch" } });
  await check();
  await respond("Download and Install");
  await settle(20);
  expect(dialogText()).toContain("The update could not be verified and was not installed.");
  expect(buttons()).toEqual(["Close"]);
});

test("an app it can't replace points to the release page", async () => {
  mock().set({ latest: "0.5.4", failInstall: { kind: "no_permission", detail: "Permission denied" } });
  await check();
  await respond("Download and Install");
  await settle(20);
  expect(dialogText()).toContain(
    "File Browser can't replace itself in this folder. Download the new version from the release page."
  );
  expect(buttons()).toEqual(["Close", "Open Release Page"]);
});

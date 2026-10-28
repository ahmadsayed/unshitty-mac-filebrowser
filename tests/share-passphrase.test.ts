// The real sharing UI against the in-memory backend. Crypto and device claims
// are covered by the Rust link/state tests; this checks the user's workflow.
import { beforeAll, beforeEach, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { promptSharePassphrase } from "../src/share-passphrase";
import {
  $, $$, boot, chooseMenu, click, closeOverlays, dialogHeading, goTo, hit, key,
  mouse, names, respond, settle, title, toasts, type,
} from "./app";

const HOME = "/Users/me";
const PHRASE = "  Violet river — 灯台  ";
const mock = () => (globalThis as unknown as {
  __mockShare: { reset(): void; protect(link: string, passphrase: string): string };
}).__mockShare;
const lastCopied = () => String(vi.mocked(navigator.clipboard.writeText).mock.calls.at(-1)?.[0]);
const remoteLink = () => mock().protect(`share://${"mockprotected".padEnd(64, "a")}`, PHRASE);

async function shareDocuments(): Promise<void> {
  await invoke("set_share_name", { name: "Ahmed" });
  await mouse(hit("Documents"), "mousedown", { button: 2 });
  await mouse(hit("Documents"), "contextmenu", { button: 2 });
  await chooseMenu("Share…");
  await settle(10);
}

async function enterProtection(): Promise<void> {
  await click($("#share-protect"));
  await type($<HTMLInputElement>("#share-passphrase"), PHRASE);
  await type($<HTMLInputElement>("#share-passphrase-confirm"), PHRASE);
}

async function pasteInNetwork(link: string): Promise<void> {
  const network = $$(".sidebar-row").find((row) => row.textContent?.trim() === "Network")!;
  await click(network);
  await type($<HTMLInputElement>(".network-link"), link);
  await click($(".network-paste .btn"));
  await settle(10);
}

beforeAll(boot);
beforeEach(async () => {
  await closeOverlays();
  mock().reset();
  await goTo(HOME);
});

test("optional protection validates confirmation and copies the protected row again", async () => {
  await shareDocuments();
  expect($<HTMLInputElement>("#share-protect").checked).toBe(false);
  expect($(".share-passphrase-fields").hidden).toBe(true);
  await click($("#share-protect"));
  expect($<HTMLInputElement>("#share-passphrase").type).toBe("password");
  expect($<HTMLInputElement>("#share-passphrase-confirm").type).toBe("password");
  await respond("Copy Link");
  expect($(".share-protection .field-message").textContent).toBe("Enter a passphrase.");
  expect($$(".share-access-row")).toHaveLength(0);
  await type($<HTMLInputElement>("#share-passphrase"), PHRASE);
  await type($<HTMLInputElement>("#share-passphrase-confirm"), "different");
  await respond("Copy Link");
  expect($(".share-protection .field-message").textContent).toBe("Passphrases do not match.");
  expect($$(".share-access-row")).toHaveLength(0);
  await type($<HTMLInputElement>("#share-passphrase-confirm"), PHRASE);
  await respond("Copy Link");
  const protectedLink = lastCopied();
  expect(protectedLink).toMatch(/^share:\/\/[a-z2-7]{64,}$/);
  expect(protectedLink).not.toContain(PHRASE);
  expect(await invoke("share_link_requires_passphrase", { link: protectedLink })).toBe(true);
  expect($(".share-access-list").textContent).toContain("Passphrase protected");
  expect($<HTMLInputElement>("#share-passphrase").value).toBe("");
  expect($<HTMLInputElement>("#share-protect").checked).toBe(true);
  await click($(".share-copy"));
  expect(lastCopied()).toBe(protectedLink);
  expect($$(".share-access-row")).toHaveLength(1);
  await respond("Close");
  await shareDocuments();
  await click($(".share-copy"));
  expect(lastCopied()).toBe(protectedLink);
  await click($(".share-remove"));
  expect($$(".share-access-row")).toHaveLength(0);
});

test("a protected invitation is separate from an existing ordinary anyone link", async () => {
  await shareDocuments();
  await respond("Copy Link");
  const ordinary = lastCopied();
  await enterProtection();
  await respond("Copy Link");
  const protectedLink = lastCopied();
  expect(protectedLink).not.toBe(ordinary);
  expect($$(".share-access-row")).toHaveLength(2);
  await click($("#share-protect"));
  await respond("Copy Link");
  expect(lastCopied()).toBe(ordinary);
  expect(await invoke("share_link_requires_passphrase", { link: ordinary })).toBe(false);
  expect($$(".share-access-row")).toHaveLength(2);
});

test("specific person links use the passphrase and Enter submits only the name prompt", async () => {
  await shareDocuments();
  await enterProtection();
  await respond("Specific Person…");
  expect(dialogHeading()).toBe("Share With a Specific Person");
  await type($<HTMLInputElement>("#share-person"), "Sara");
  await key("Enter", { target: $("#share-person") });
  await settle(20);
  expect(dialogHeading()).toBe("Share “Documents”");
  expect($(".share-access-list").textContent).toContain("Sara");
  expect($(".share-access-list").textContent).toContain("Passphrase protected");
  expect(await invoke("share_link_requires_passphrase", { link: lastCopied() })).toBe(true);
  const settings = await invoke<{ access: Array<{ kind: string; protected: boolean }> }>("share_info", { path: `${HOME}/Documents` });
  expect(settings.access).toEqual([expect.objectContaining({ kind: "person", protected: true })]);
});

test("Escape cancels only the topmost sharing confirmation", async () => {
  await shareDocuments();
  await enterProtection();
  await respond("Stop Sharing…");
  expect(dialogHeading()).toBe("Stop sharing “Documents”?");
  await key("Escape");
  expect(dialogHeading()).toBe("Share “Documents”");
  expect($<HTMLInputElement>("#share-passphrase").value).toBe(PHRASE);
  expect(await invoke<unknown[]>("list_shares")).toHaveLength(1);
});

test("Network detects protected links, retries a wrong passphrase and adds only after unlocking", async () => {
  const link = remoteLink();
  await pasteInNetwork(`<${link.toUpperCase()}/>`);
  expect(dialogHeading()).toBe("Enter Passphrase");
  const input = $<HTMLInputElement>("#share-unlock-passphrase");
  expect(input.type).toBe("password");
  expect(await invoke("list_received")).toEqual([]);
  await type(input, "Ahmed");
  await respond("Unlock");
  expect(dialogHeading()).toBe("Enter Passphrase");
  expect($(".dialog-extra .field-message").textContent).toBe("Incorrect passphrase or damaged share link.");
  expect(await invoke("list_received")).toEqual([]);
  await type(input, PHRASE);
  await key("Enter", { target: input });
  await settle(20);
  expect(dialogHeading()).toBeUndefined();
  expect(input.value).toBe("");
  expect(title()).toBe("Network");
  expect(names()).toEqual(["Photos"]);
  expect(await invoke<unknown[]>("list_received")).toHaveLength(1);
  expect(toasts().join(" ")).not.toContain(PHRASE);
  expect(localStorage.getItem("prefs") ?? "").not.toContain(PHRASE);
});

test("cancelling the Network prompt clears the secret and adds nothing", async () => {
  await pasteInNetwork(remoteLink());
  const input = $<HTMLInputElement>("#share-unlock-passphrase");
  await type(input, PHRASE);
  await key("Escape", { target: input });
  expect(dialogHeading()).toBeUndefined();
  expect(input.value).toBe("");
  expect(await invoke("list_received")).toEqual([]);
  expect($<HTMLButtonElement>(".network-paste .btn").disabled).toBe(false);
  expect($(".network-error").textContent).toBe("");
});

test("the location bar automatically unlocks protected links and can be cancelled", async () => {
  const link = remoteLink();
  await goTo(link);
  expect(dialogHeading()).toBe("Enter Passphrase");
  await respond("Cancel");
  expect(await invoke("list_received")).toEqual([]);
  expect(title()).toBe("Home");
  await key("Enter", { target: $("#path-entry") });
  expect(dialogHeading()).toBe("Enter Passphrase");
  await type($<HTMLInputElement>("#share-unlock-passphrase"), PHRASE);
  await respond("Unlock");
  expect(title()).toBe("Photos");
  expect($("#share-subtitle").textContent).toContain("Read only");
});

test("your own protected link unlocks to its local folder", async () => {
  await shareDocuments();
  await enterProtection();
  await respond("Copy Link");
  const link = lastCopied();
  await respond("Close");
  await pasteInNetwork(link);
  expect(dialogHeading()).toBe("Enter Passphrase");
  await type($<HTMLInputElement>("#share-unlock-passphrase"), PHRASE);
  await respond("Unlock");
  expect(title()).toBe("Documents");
  expect(await invoke("list_received")).toEqual([]);
});

test("oversized passphrases stay inline and toggling protection clears both fields", async () => {
  await shareDocuments();
  await enterProtection();
  await type($<HTMLInputElement>("#share-passphrase"), "灯".repeat(342));
  await respond("Copy Link");
  expect($(".share-protection .field-message").textContent).toContain("at most 1,024 bytes");
  expect($$(".share-access-row")).toHaveLength(0);
  await click($("#share-protect"));
  expect($<HTMLInputElement>("#share-passphrase").value).toBe("");
  expect($<HTMLInputElement>("#share-passphrase-confirm").value).toBe("");
  expect($(".share-passphrase-fields").hidden).toBe(true);
});

test("an in-flight unlock cannot submit twice or report cancellation while claiming", async () => {
  let finish!: (value: string) => void;
  const pending = new Promise<string>((resolve) => { finish = resolve; });
  const submit = vi.fn(() => pending);
  const result = promptSharePassphrase(submit);
  const input = $<HTMLInputElement>("#share-unlock-passphrase");
  await type(input, PHRASE);
  await respond("Unlock");
  expect(input.disabled).toBe(true);
  await key("Enter", { target: input });
  await key("Escape", { target: input });
  expect(submit).toHaveBeenCalledTimes(1);
  expect(dialogHeading()).toBe("Enter Passphrase");
  finish("opened");
  expect(await result).toBe("opened");
  expect(input.value).toBe("");
  expect(dialogHeading()).toBeUndefined();
});

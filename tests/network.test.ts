// Network: SSH/SFTP connection, host-key trust, password fallback and browsing.
// Received shares in the same place are covered by share.test.ts.
import { beforeAll, expect, test } from "vitest";
import {
  $, $$, boot, chooseMenu, click, dialogHeading, item, key, menuLabels, mouse, names, respond, settle, title, toasts, type,
} from "./app";

const sftp = () =>
  (globalThis as unknown as {
    __mockSftp: { reset(): void; requirePassword(): void; unknownHost(): void };
  }).__mockSftp;

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

const place = (label: string) => $$(".sidebar-row").find((row) => row.textContent?.trim() === label)!;

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

/** Type an address into Network's bar and press Connect. */
async function enterAddress(address: string): Promise<void> {
  await type($<HTMLInputElement>(".network-link"), address);
  await click($(".network-paste .btn"));
  await settle(30);
}

test("Network connects to an SSH server and browses it read-only", async () => {
  sftp().reset();
  await click(place("Network"));
  await settle(20);
  expect(title()).toBe("Network");
  expect($("#view").textContent).toContain("No Network Locations");

  // A server address connects straight away when an SSH key works.
  await enterAddress("user@host");
  expect(title()).toBe("user@host");
  expect(names()).toEqual(["Documents", "notes.txt"]);
  expect($("#share-subtitle").textContent).toBe("SSH server · Read only");

  await click($$(".item").find((item) => item.textContent?.includes("Documents"))!);
  await key("Enter");
  await settle(20);
  expect(title()).toBe("Documents");
  expect(names()).toEqual(["hello.txt"]);
});

test("Network lists a connected server and can disconnect it", async () => {
  await click(place("Network"));
  await settle(20);
  expect(names()).toEqual(["user@host"]);
  expect($$(".shared-by-cell").map((cell) => cell.textContent)).toEqual(["SSH server"]);
  expect($$(".network-status").map((cell) => cell.textContent)).toEqual(["Connected"]);

  await rightClick(item("user@host"));
  expect(menuLabels()).toEqual(["Open", "Copy Location", "Disconnect"]);
  await chooseMenu("Disconnect");
  await settle(20);
  expect(toasts().join(" | ")).toContain("Disconnected from user@host");
  expect($("#view").textContent).toContain("No Network Locations");
});

test("a password is requested when no key works", async () => {
  sftp().reset();
  sftp().requirePassword();
  await click(place("Network"));
  await settle(20);
  await rightClick($("#view"));
  await chooseMenu("Connect to Server…");
  await settle(10);
  expect(dialogHeading()).toBe("Connect to Server");
  await type($<HTMLInputElement>("#sftp-server"), "user@host");
  await respond("Connect");
  await settle(20);
  expect($(".dialog-layer").textContent).toContain("No SSH key was accepted");

  await type($<HTMLInputElement>("#sftp-password"), "secret");
  await respond("Connect");
  await settle(30);
  expect(title()).toBe("user@host");
  expect(names()).toEqual(["Documents", "notes.txt"]);
});

test("an unknown host key asks before connecting", async () => {
  sftp().reset();
  sftp().unknownHost();
  await click(place("Network"));
  await settle(20);
  await enterAddress("sftp://user@host");
  expect(dialogHeading()).toBe("Trust This Server?");
  expect($$(".dialog-layer").at(-1)!.textContent).toContain("SHA256:mock-fingerprint");

  await respond("Trust");
  await settle(30);
  expect(title()).toBe("user@host");
  expect(names()).toEqual(["Documents", "notes.txt"]);
});

// Network: SSH/SFTP connection, host-key trust, password fallback and browsing.
import { beforeAll, expect, test } from "vitest";
import { $, $$, boot, click, dialogHeading, key, names, respond, settle, title, type } from "./app";

const sftp = () =>
  (globalThis as unknown as {
    __mockSftp: { reset(): void; requirePassword(): void; unknownHost(): void };
  }).__mockSftp;

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

const place = (label: string) => $$(".sidebar-row").find((row) => row.textContent?.trim() === label)!;

test("Network connects to an SSH server and browses it read-only", async () => {
  sftp().reset();
  await click(place("Network"));
  await settle(20);
  expect(title()).toBe("Network");
  expect($("#view").textContent).toContain("No Servers");

  await click($(".server-connect"));
  await settle(10);
  expect(dialogHeading()).toBe("Connect to Server");
  await type($<HTMLInputElement>("#sftp-server"), "user@host");
  await respond("Connect");
  await settle(30);
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
  expect($(".server-name").textContent).toBe("user@host");
  await click($(".server-disconnect"));
  await settle(20);
  expect($("#view").textContent).toContain("No Servers");
});

test("a password is requested when no key works", async () => {
  sftp().reset();
  sftp().requirePassword();
  await click(place("Network"));
  await settle(20);
  await click($(".server-connect"));
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
  await click($(".server-connect"));
  await type($<HTMLInputElement>("#sftp-server"), "user@host");
  await respond("Connect");
  await settle(20);
  expect(dialogHeading()).toBe("Trust This Server?");
  expect($$(".dialog-layer").at(-1)!.textContent).toContain("SHA256:mock-fingerprint");

  await respond("Trust");
  await settle(30);
  expect(title()).toBe("user@host");
  expect(names()).toEqual(["Documents", "notes.txt"]);
});

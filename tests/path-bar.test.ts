// The path bar's crumbs, Up and the window title for every kind of path:
// local folders, Home, the Trash, mounted volumes, received shares and SSH
// servers. Characterization tests: they pin what the app does today so the
// path rules can move into src/paths.ts without anyone seeing a change.
import { beforeAll, expect, test } from "vitest";
import { $, $$, boot, click, goTo, item, key, names, selectedNames, settle, title, type } from "./app";

const HOME = "/Users/me";

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

/** Each crumb as [title, path]: the title is its label, or "Computer" for "/". */
function crumbs(): [string, string][] {
  return $$("#crumbs .crumb").map((c) => [c.title, c.dataset.path ?? ""]);
}

const place = (label: string) => $$(".sidebar-row").find((row) => row.textContent?.trim() === label)!;

async function up(): Promise<void> {
  await key("ArrowUp", { alt: true });
  await settle(20);
}

async function openItem(name: string): Promise<void> {
  await click(item(name).querySelector<HTMLElement>(".icon")!);
  await key("Enter");
  await settle(20);
}

test("a folder in Home has a Home crumb, then one per folder", async () => {
  await goTo(`${HOME}/Documents/Projects`);
  expect(crumbs()).toEqual([
    ["Home", HOME],
    ["Documents", `${HOME}/Documents`],
    ["Projects", `${HOME}/Documents/Projects`],
  ]);
  expect(title()).toBe("Projects");
});

test("a folder outside Home starts at the computer's root", async () => {
  await goTo("/Applications");
  expect(crumbs()).toEqual([
    ["Computer", "/"],
    ["Applications", "/Applications"],
  ]);
  await goTo("/");
  expect(crumbs()).toEqual([["Computer", "/"]]);
});

test("a mounted volume starts at its own crumb", async () => {
  await goTo("/Volumes/Backup Drive");
  expect(crumbs()).toEqual([["Backup Drive", "/Volumes/Backup Drive"]]);
});

test("the Trash has its own crumb", async () => {
  await goTo(`${HOME}/.Trash`);
  expect(crumbs()).toEqual([["Trash", `${HOME}/.Trash`]]);
  expect(title()).toBe("Trash");
});

test("Up goes to the parent folder and selects where it came from, and stops at /", async () => {
  await goTo(`${HOME}/Documents/Projects`);
  await up();
  expect(crumbs().at(-1)).toEqual(["Documents", `${HOME}/Documents`]);
  expect(selectedNames()).toEqual(["Projects"]);
  await goTo("/");
  await up();
  expect(crumbs()).toEqual([["Computer", "/"]]);
});

test("a received share's crumbs start at the sharer's folder name; Up returns to Network", async () => {
  (globalThis as unknown as { __mockShare: { reset(): void } }).__mockShare.reset();
  await click(place("Network"));
  await settle(10);
  await type($<HTMLInputElement>(".network-link"), `share://${"crumbs".padEnd(64, "a")}`);
  await click($(".network-paste .btn"));
  await settle(20);
  expect(names()).toEqual(["Photos"]);

  await openItem("Photos");
  expect(crumbs()).toEqual([["Photos", "share://r1"]]);
  expect(title()).toBe("Photos");

  await openItem("Photos");
  expect(crumbs()).toEqual([
    ["Photos", "share://r1"],
    ["Photos", "share://r1/Photos"],
  ]);
  expect(title()).toBe("Photos");

  await up();
  expect(crumbs()).toEqual([["Photos", "share://r1"]]);
  expect(selectedNames()).toEqual(["Photos"]);
  await up();
  expect(title()).toBe("Network");
  expect(selectedNames()).toEqual(["Photos"]);
});

test("an SSH server's crumbs start at the server; Up from its root selects it in Network", async () => {
  await click(place("Network"));
  await settle(10);
  await type($<HTMLInputElement>(".network-link"), "user@host");
  await click($(".network-paste .btn"));
  await settle(30);
  // A server starts in the user's home on it, as sftp.rs connects, and its
  // Network entry is that folder, not the root.
  expect(crumbs()).toEqual([
    ["user@host", "sftp://user@host/"],
    ["home", "sftp://user@host/home"],
    ["user", "sftp://user@host/home/user"],
  ]);
  expect(title()).toBe("user");

  await openItem("Documents");
  expect(crumbs().at(-1)).toEqual(["Documents", "sftp://user@host/home/user/Documents"]);
  expect(title()).toBe("Documents");

  await $$("#crumbs .crumb")[0].click();
  await settle(20);
  expect(names()).toEqual(["home"]);
  await up();
  expect(title()).toBe("Network");
  expect(selectedNames()).toEqual(["user@host"]);
});

test("Up from an SSH subfolder reaches the server's root; Up again selects it in Network", async () => {
  await openItem("user@host");
  await openItem("Documents");
  await up();
  expect(names()).toEqual(["Documents", "notes.txt"]);
  expect(selectedNames()).toEqual(["Documents"]);
  await up();
  await up();
  expect(selectedNames()).toEqual(["home"]);
  expect(crumbs()).toEqual([["user@host", "sftp://user@host/"]]);
  await up();
  expect(title()).toBe("Network");
  expect(selectedNames()).toEqual(["user@host"]);
});

test("a server's root typed with or without its slash is the same place", async () => {
  for (const typed of ["sftp://user@host", "sftp://user@host/", "sftp://user@host//"]) {
    await goTo(typed);
    expect(crumbs()).toEqual([["user@host", "sftp://user@host/"]]);
    await up();
    expect(title()).toBe("Network");
    expect(selectedNames()).toEqual(["user@host"]);
  }
  // "/" typed on a server is its root too.
  await goTo("sftp://user@host/home/user/Documents");
  await goTo("/");
  expect(crumbs()).toEqual([["user@host", "sftp://user@host/"]]);
  await up();
  expect(selectedNames()).toEqual(["user@host"]);
});

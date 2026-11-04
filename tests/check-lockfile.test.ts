// The release workflow refuses a package-lock.json that could install anything
// but the public npm registry's own tarballs (scripts/check-lockfile.mjs).
import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { checkLockfile } from "../scripts/check-lockfile.mjs";

const REGISTRY = "https://registry.npmjs.org/";
const SHA = `sha512-${"A".repeat(86)}==`;
const entry = (name: string, version: string) => ({
  version,
  resolved: `${REGISTRY}${name}/-/${name.split("/").pop()}-${version}.tgz`,
  integrity: SHA,
});
const lockOf = (packages: Record<string, object>) => ({
  lockfileVersion: 3,
  packages: { "": { name: "filebrowser" }, ...packages },
});
const pkg = { dependencies: { vite: "^8" }, devDependencies: { "@tauri-apps/cli": "^2" } };
const clean = () =>
  lockOf({
    "node_modules/vite": entry("vite", "8.3.5"),
    "node_modules/@tauri-apps/cli": entry("@tauri-apps/cli", "2.12.4"),
    "node_modules/vite/node_modules/picocolors": entry("picocolors", "1.1.1"),
  });

test("the committed lockfile passes", () => {
  const lock = JSON.parse(readFileSync("package-lock.json", "utf8"));
  const pkgJson = JSON.parse(readFileSync("package.json", "utf8"));
  expect(checkLockfile({ lock, pkg: pkgJson, npmrcs: {} })).toEqual([]);
});

test("a clean lock, including nested and scoped packages, passes", () => {
  expect(checkLockfile({ lock: clean(), pkg, npmrcs: {} })).toEqual([]);
});

test("another registry host is refused (the old corporate lockfile)", () => {
  const lock = clean();
  (lock.packages as Record<string, { resolved: string }>)["node_modules/vite"].resolved =
    "https://npm.corp.example/vite/-/vite-8.3.5.tgz";
  expect(checkLockfile({ lock, pkg, npmrcs: {} })).toEqual([
    `node_modules/vite: resolved https://npm.corp.example/vite/-/vite-8.3.5.tgz, expected ${REGISTRY}vite/-/vite-8.3.5.tgz`,
  ]);
});

test("an entry pointing at another package's tarball is refused (lockfile injection)", () => {
  const lock = clean();
  (lock.packages as Record<string, { resolved: string }>)["node_modules/vite/node_modules/picocolors"].resolved =
    `${REGISTRY}evil-pkg/-/evil-pkg-1.0.0.tgz`;
  expect(checkLockfile({ lock, pkg, npmrcs: {} })[0]).toMatch(/picocolors: resolved .*evil-pkg/);
});

test("missing or weak integrity, links and odd keys are refused", () => {
  const lock = clean();
  const packages = lock.packages as Record<string, Record<string, unknown>>;
  packages["node_modules/vite"].integrity = "sha1-abc";
  packages["node_modules/linked"] = { link: true, resolved: "../linked" };
  packages["vendor/thing"] = entry("thing", "1.0.0");
  expect(checkLockfile({ lock, pkg, npmrcs: {} })).toEqual([
    "node_modules/vite: missing or non-sha512 integrity",
    "node_modules/linked: link entries are not allowed",
    "vendor/thing: not under node_modules",
  ]);
});

test("only lockfile version 3 is accepted", () => {
  expect(checkLockfile({ lock: { ...clean(), lockfileVersion: 2 }, pkg, npmrcs: {} })).toEqual([
    "lockfileVersion is 2, expected 3",
  ]);
});

test("an .npmrc may name only the public registry, and never credentials", () => {
  const check = (text: string) => checkLockfile({ lock: clean(), pkg, npmrcs: { ".npmrc": text } });
  expect(check(`registry=${REGISTRY}\n# a comment\nfund=false\n`)).toEqual([]);
  expect(check("registry=https://npm.corp.example/\n")).toEqual([".npmrc: registry=https://npm.corp.example/"]);
  expect(check("@acme:registry=https://npm.acme.example/\n")).toEqual([".npmrc: @acme:registry=https://npm.acme.example/"]);
  expect(check("//registry.npmjs.org/:_authToken=abc\n")).toEqual([".npmrc: sets credentials"]);
  expect(check("always-auth=true\n")).toEqual([".npmrc: sets credentials"]);
});

test("package.json may only use registry ranges", () => {
  const bad = {
    dependencies: { a: "git+https://github.com/x/a.git", b: "file:../b", c: "https://x.example/c.tgz", d: "user/repo" },
    publishConfig: { registry: "https://npm.corp.example/" },
  };
  expect(checkLockfile({ lock: clean(), pkg: bad, npmrcs: {} })).toEqual([
    "package.json: publishConfig.registry is set",
    'package.json dependencies.a: "git+https://github.com/x/a.git" is not a registry range',
    'package.json dependencies.b: "file:../b" is not a registry range',
    'package.json dependencies.c: "https://x.example/c.tgz" is not a registry range',
    'package.json dependencies.d: "user/repo" is not a registry range',
  ]);
});

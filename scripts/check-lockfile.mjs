#!/usr/bin/env node
// Refuse a package-lock.json that could install anything but the public npm
// registry's own tarball for each name and version it lists. The release
// workflow runs it before `npm ci`:
//
//   node scripts/check-lockfile.mjs [repo-root]
//
// - every entry resolves to https://registry.npmjs.org/<name>/-/<name>-<v>.tgz
//   (another host, like the corporate lockfile this repo once had, or another
//   package's tarball, as in a lockfile injection, is refused) and has a
//   sha512 integrity hash, which `npm ci` checks the download against;
// - no link entries and no git, file or URL dependencies in package.json;
// - an .npmrc may name only the public registry and never sets credentials.
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const REGISTRY = "https://registry.npmjs.org/";
const NPMRCS = [".npmrc", "src-tauri/.npmrc", "web/file-get/.npmrc"];

function checkEntry(key, entry) {
  if (entry.link) return [`${key}: link entries are not allowed`];
  if (!key.includes("node_modules/")) return [`${key}: not under node_modules`];
  const name = entry.name ?? key.slice(key.lastIndexOf("node_modules/") + "node_modules/".length);
  const expected = `${REGISTRY}${name}/-/${name.split("/").pop()}-${entry.version}.tgz`;
  const errors = [];
  if (entry.resolved !== expected) errors.push(`${key}: resolved ${entry.resolved ?? "(missing)"}, expected ${expected}`);
  if (!/^sha512-[A-Za-z0-9+/]{86}==$/.test(entry.integrity ?? "")) errors.push(`${key}: missing or non-sha512 integrity`);
  return errors;
}

function checkNpmrc(file, text) {
  const errors = [];
  for (const line of text.split("\n").map((l) => l.trim())) {
    if (!line || line.startsWith("#") || line.startsWith(";")) continue;
    if (/_auth|_password|username|always-auth|certfile|keyfile/i.test(line)) errors.push(`${file}: sets credentials`);
    else if (/registry\s*=/i.test(line) && line.replace(/\s/g, "") !== `registry=${REGISTRY}`) errors.push(`${file}: ${line}`);
  }
  return errors;
}

function checkPackageJson(pkg) {
  const errors = [];
  if (pkg.publishConfig?.registry) errors.push("package.json: publishConfig.registry is set");
  for (const field of ["dependencies", "devDependencies", "optionalDependencies"]) {
    for (const [dep, spec] of Object.entries(pkg[field] ?? {})) {
      if (/^(git|github:|file:|link:|https?:)|\//.test(spec)) errors.push(`package.json ${field}.${dep}: "${spec}" is not a registry range`);
    }
  }
  return errors;
}

/** Every problem with `lock` (package-lock.json), `pkg` (package.json) and
 * `npmrcs` ({ path: text }), or [] when installs can only come from the
 * public registry. */
export function checkLockfile({ lock, pkg, npmrcs }) {
  if (lock.lockfileVersion !== 3) return [`lockfileVersion is ${lock.lockfileVersion}, expected 3`];
  return [
    ...Object.entries(lock.packages ?? {})
      .filter(([key]) => key !== "")
      .flatMap(([key, entry]) => checkEntry(key, entry)),
    ...Object.entries(npmrcs).flatMap(([file, text]) => checkNpmrc(file, text)),
    ...checkPackageJson(pkg),
  ];
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const root = process.argv[2] ?? ".";
  const read = (file) => readFileSync(join(root, file), "utf8");
  const npmrcs = Object.fromEntries(NPMRCS.filter((f) => existsSync(join(root, f))).map((f) => [f, read(f)]));
  const lock = JSON.parse(read("package-lock.json"));
  const errors = checkLockfile({ lock, pkg: JSON.parse(read("package.json")), npmrcs });
  if (errors.length) {
    for (const error of errors) console.error(`::error file=package-lock.json::${error}`);
    console.error(`check-lockfile: ${errors.length} problem(s); regenerate with --registry=${REGISTRY} (AGENTS.md)`);
    process.exit(1);
  }
  console.log(`package-lock.json: ${Object.keys(lock.packages).length - 1} packages, all from ${REGISTRY} with sha512 integrity`);
}

#!/usr/bin/env node
// latest.json for Check for Updates: which signed package each platform
// installs. Run by the release workflow after the builds:
//
//   node scripts/updater-manifest.mjs [--require-signed] <packages-dir> <out.json>
//
// Version from package.json, tag from GITHUB_REF_NAME, repo from
// GITHUB_REPOSITORY. Without .sig files (no signing key) it writes nothing,
// and with --require-signed (the workflow passes it for a tag push) it
// fails; an incomplete set always fails, so a release never ships a manifest
// that leaves one platform with "Could not check for updates".
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { pathToFileURL } from "node:url";

/** GitHub stores an uploaded file's spaces as dots. */
const assetName = (path) => basename(path).replaceAll(" ", ".");

/** The one package of `files` ending in `suffix`, and its signature; throws
 * when either is missing. */
function signedPackage(files, suffix, what) {
  const pkg = files.find((f) => f.endsWith(suffix));
  if (!pkg) throw new Error(`No ${what} (*${suffix}) among the signed packages.`);
  if (!files.includes(`${pkg}.sig`)) throw new Error(`Missing signature ${basename(pkg)}.sig for ${what}.`);
  return pkg;
}

/**
 * The manifest for `files` (package paths, .sig files among them), or null
 * when nothing is signed. `readSig` returns a .sig file's contents.
 */
export function buildManifest(files, readSig, { version, tag, repo, notes, pubDate }) {
  const sigs = files.filter((f) => f.endsWith(".sig"));
  if (sigs.length === 0) return null;
  for (const sig of sigs) {
    const pkg = sig.slice(0, -".sig".length);
    if (!files.includes(pkg)) throw new Error(`Signature ${basename(sig)} has no package ${basename(pkg)}.`);
  }
  const entry = (pkg) => ({
    url: `https://github.com/${repo}/releases/download/${tag}/${assetName(pkg)}`,
    signature: readSig(`${pkg}.sig`),
  });
  const appImage = entry(signedPackage(files, ".AppImage", "AppImage"));
  // The Mac build is Apple Silicon only: Intel Macs get no entry.
  const mac = entry(signedPackage(files, ".app.tar.gz", "macOS app archive"));
  return {
    version,
    notes,
    pub_date: pubDate,
    platforms: { "linux-x86_64": appImage, "darwin-aarch64": mac },
  };
}

/** What to do when nothing is signed: fail when the release must be
 * signed (a tag push), else just say so. */
export function unsignedOutcome(required) {
  if (required) {
    return {
      exitCode: 1,
      message:
        "No signed packages, so no latest.json: installed copies couldn't check for updates. " +
        "Set the TAURI_SIGNING_PRIVATE_KEY and TAURI_SIGNING_PRIVATE_KEY_PASSWORD secrets and push the tag again.",
    };
  }
  return { exitCode: 0, message: "No signed packages: no latest.json (the signing key secret isn't set)." };
}

function listFiles(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? listFiles(path) : [path];
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2);
  const required = args.includes("--require-signed");
  const [dir, out] = args.filter((arg) => arg !== "--require-signed");
  if (!dir || !out) {
    console.error("usage: node scripts/updater-manifest.mjs [--require-signed] <packages-dir> <out.json>");
    process.exit(2);
  }
  const { version } = JSON.parse(readFileSync("package.json", "utf8"));
  try {
    const manifest = buildManifest(listFiles(dir), (path) => readFileSync(path, "utf8").trim(), {
      version,
      tag: process.env.GITHUB_REF_NAME ?? `v${version}`,
      repo: process.env.GITHUB_REPOSITORY ?? "ahmadsayed/unshitty-mac-filebrowser",
      notes: "See the release page for what's new.",
      pubDate: new Date().toISOString(),
    });
    if (!manifest) {
      const { exitCode, message } = unsignedOutcome(required);
      (exitCode ? console.error : console.log)(message);
      process.exitCode = exitCode;
    } else {
      writeFileSync(out, `${JSON.stringify(manifest, null, 2)}\n`);
      console.log(`Wrote ${out} for ${version}.`);
    }
  } catch (error) {
    console.error(`updater-manifest: ${error.message}`);
    process.exit(1);
  }
}

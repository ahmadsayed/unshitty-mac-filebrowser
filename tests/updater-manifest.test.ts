// latest.json for Check for Updates, written by the release workflow from the
// signed packages (scripts/updater-manifest.mjs).
import { expect, test } from "vitest";
import { buildManifest } from "../scripts/updater-manifest.mjs";

const opts = { version: "0.5.4", tag: "v0.5.4", repo: "o/r", notes: "See the release page.", pubDate: "2026-11-01T10:00:00Z" };
const sigOf = (path: string) => `sig of ${path.split("/").pop()}`;

test("writes all three platforms with GitHub's asset names", () => {
  const files = [
    "a/File Browser_0.5.4_amd64.AppImage",
    "a/File Browser_0.5.4_amd64.AppImage.sig",
    "b/File Browser.app.tar.gz",
    "b/File Browser.app.tar.gz.sig",
    "c/File.Browser_0.5.4_amd64.deb",
  ];
  const manifest = buildManifest(files, sigOf, opts) as {
    version: string; notes: string; pub_date: string; platforms: Record<string, { url: string; signature: string }>;
  };
  expect(manifest.version).toBe("0.5.4");
  expect(manifest.notes).toBe("See the release page.");
  expect(manifest.pub_date).toBe("2026-11-01T10:00:00Z");
  expect(Object.keys(manifest.platforms).sort()).toEqual(["darwin-aarch64", "darwin-x86_64", "linux-x86_64"]);
  expect(manifest.platforms["linux-x86_64"]).toEqual({
    url: "https://github.com/o/r/releases/download/v0.5.4/File.Browser_0.5.4_amd64.AppImage",
    signature: "sig of File Browser_0.5.4_amd64.AppImage.sig",
  });
  expect(manifest.platforms["darwin-aarch64"]).toEqual({
    url: "https://github.com/o/r/releases/download/v0.5.4/File.Browser.app.tar.gz",
    signature: "sig of File Browser.app.tar.gz.sig",
  });
  expect(manifest.platforms["darwin-x86_64"]).toEqual(manifest.platforms["darwin-aarch64"]);
});

test("no signatures, no manifest", () => {
  expect(buildManifest(["a/File.Browser_0.5.4_amd64.AppImage", "b/x.dmg"], sigOf, opts)).toBeNull();
});

test("an incomplete set fails instead of shipping a partial manifest", () => {
  const appImage = ["a/File Browser_0.5.4_amd64.AppImage", "a/File Browser_0.5.4_amd64.AppImage.sig"];
  // The Mac archive is there but unsigned.
  expect(() => buildManifest([...appImage, "b/File Browser.app.tar.gz"], sigOf, opts)).toThrow(/File Browser\.app\.tar\.gz\.sig/);
  // No Mac archive at all.
  expect(() => buildManifest(appImage, sigOf, opts)).toThrow(/\.app\.tar\.gz/);
  // A signature without its package.
  expect(() =>
    buildManifest([...appImage, "b/File Browser.app.tar.gz.sig"], sigOf, opts)
  ).toThrow(/File Browser\.app\.tar\.gz\b/);
});

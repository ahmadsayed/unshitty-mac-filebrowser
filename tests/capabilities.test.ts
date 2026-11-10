// What each kind of place allows, as the backend's providers say it
// (src/capabilities.ts, design D5). tests/capabilities.json holds the
// backend's table; a Rust test in src-tauri/src/providers.rs checks the
// backend against it, tests/mock-core.test.ts the mock, and these the
// frontend's built-in table and the rows it picks for each path.
import { afterEach, expect, test } from "vitest";
import fixture from "./capabilities.json";
import {
  BUILT_IN_CAPABILITIES,
  capabilitiesOf,
  isCapabilityTable,
  loadCapabilities,
  resetCapabilities,
  setCapabilities,
  type CapabilityRow,
  type CapabilityTable,
} from "../src/capabilities";
import { capabilityRow, isReadOnlyKind, kindAcceptsNewItems, kindTakesPastes, knowsSizes, locationKind, searchesRecursively, takesDrops } from "../src/locations";
import { refusal } from "../src/item-actions";

afterEach(resetCapabilities);

const table = fixture.table as CapabilityTable;

test("the built-in table is the backend's", () => {
  expect(BUILT_IN_CAPABILITIES).toEqual(table);
  expect(isCapabilityTable(table)).toBe(true);
});

test("each row's path picks that row", () => {
  for (const [row, path] of Object.entries(fixture.paths)) {
    expect(capabilityRow(locationKind(path, ""), path), path).toBe(row as CapabilityRow);
  }
});

test("the frontend's own views have no row", () => {
  for (const kind of ["recent", "starred", "network", "trash", "in-trash"] as const) {
    expect(capabilityRow(kind, "/Users/me")).toBeNull();
  }
  // An Arbesk item whose path isn't known is taken as a collection, and
  // anything but a collection or an asset as the account.
  expect(capabilityRow("arbesk")).toBe("arbesk-collection");
  expect(capabilityRow("arbesk", "arbesk://")).toBe("arbesk-account");
});

test("only a whole table replaces the one there is", () => {
  const partial = { ...table, drive: { ...table.drive, can_trash: "yes" } };
  setCapabilities(partial);
  expect(capabilitiesOf("drive").can_trash).toBe(true);
  setCapabilities({ folder: table.folder });
  setCapabilities(null);
  expect(capabilitiesOf("folder")).toEqual(table.folder);
});

test("a failed read keeps the built-in table", async () => {
  await loadCapabilities(() => Promise.reject("no backend"));
  expect(capabilitiesOf("sftp")).toEqual(BUILT_IN_CAPABILITIES.sftp);
});

test("the rules read the table the backend gave", async () => {
  const changed: CapabilityTable = structuredClone(table);
  changed.drive = { ...changed.drive, read_only: true, can_trash: false, can_rename: false, knows_sizes: false, searchable: true };
  await loadCapabilities(async () => changed);
  expect(isReadOnlyKind("drive")).toBe(true);
  expect(kindAcceptsNewItems("drive")).toBe(false);
  expect(knowsSizes("drive")).toBe(false);
  expect(searchesRecursively("gdrive://acct/root~x", "")).toBe(true);
  expect(refusal("trash", { kind: "drive", items: [] })).toBe("");
  expect(refusal("rename", { kind: "folder", items: ["drive"] })).toBe("");
  resetCapabilities();
  expect(refusal("trash", { kind: "drive", items: [] })).toBeNull();
});

test("paste reads can_copy_in, New Folder can_create", async () => {
  const changed: CapabilityTable = structuredClone(table);
  changed.drive = { ...changed.drive, can_copy_in: false };
  changed.folder = { ...changed.folder, can_create: false };
  await loadCapabilities(async () => changed);
  expect(refusal("paste", { kind: "drive", items: [] })).toBe("");
  expect(kindTakesPastes("drive")).toBe(false);
  expect(kindAcceptsNewItems("drive")).toBe(true);
  expect(refusal("paste", { kind: "folder", items: [] })).toBeNull();
  expect(kindAcceptsNewItems("folder")).toBe(false);
});

test("a drop reads can_copy_in, as Paste does", async () => {
  const drive = "gdrive://acct/root~x";
  expect(takesDrops(drive)).toBe(true);
  expect(takesDrops("/Users/me/Documents")).toBe(true);
  expect(takesDrops("sftp://me@host/home")).toBe(false);
  const changed: CapabilityTable = structuredClone(table);
  changed.drive = { ...changed.drive, can_copy_in: false };
  await loadCapabilities(async () => changed);
  expect(takesDrops(drive)).toBe(false);
  expect(takesDrops("/Users/me/Documents")).toBe(true);
});

test("cut's read-only toast is picked by the item's kind", async () => {
  const changed: CapabilityTable = structuredClone(table);
  changed.sftp = { ...changed.sftp, can_copy_out: true };
  await loadCapabilities(async () => changed);
  expect(refusal("cut", { kind: "sftp", items: ["sftp"], paths: ["sftp://me@host/home/a"] })).toBe("Shared folders are read only.");
  const asset = "arbesk://0x00000000000000000000000000000000000000ab/12~Set/a1~Box.gltf";
  expect(refusal("cut", { kind: "arbesk", items: ["arbesk"], paths: [asset] })).toBe("Arbesk is read only.");
});

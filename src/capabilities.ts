// What each kind of place allows, as the backend's places say it (design D5):
// the booleans of each provider's `Capabilities`, one row per kind of place
// the frontend tells apart, read once at startup (`loadCapabilities`) into a
// synchronous cache, so `locations.ts` and `item-actions.ts` stay pure
// functions. The table is static per kind and asks no server.
//
// Until the backend's answer arrives (and if it never does) the built-in
// table answers: today's answers, the same as the backend's.
// `tests/capabilities.json` pins the backend's table (a Rust test in
// `providers.rs`), this one and the mock's (`tests/capabilities.test.ts`),
// so the three can't drift.

/** What a kind of place allows: the booleans of the backend's
 *  `places::Capabilities` for it (`can_copy_in`: no copy-in refusal). */
export interface PlaceCapabilities {
  read_only: boolean;
  can_create: boolean;
  can_rename: boolean;
  can_trash: boolean;
  can_copy_in: boolean;
  can_copy_out: boolean;
  knows_sizes: boolean;
  knows_counts: boolean;
  searchable: boolean;
  has_thumbnails: boolean;
}

/** The kinds of place the backend answers for: a folder on this computer,
 *  Drive, a received share, an SSH server, and Arbesk's account, collection,
 *  asset and an asset's other file, which allow different things. Recent, Starred, Network and
 *  the Trash are the frontend's own views and have no row
 *  (`locations.ts`' `capabilityRow`). */
export type CapabilityRow = "folder" | "drive" | "share" | "sftp" | "arbesk-account" | "arbesk-collection" | "arbesk-asset" | "arbesk-file";

export type CapabilityTable = Record<CapabilityRow, PlaceCapabilities>;

const ROWS: readonly CapabilityRow[] = ["folder", "drive", "share", "sftp", "arbesk-account", "arbesk-collection", "arbesk-asset", "arbesk-file"];
const FLAGS: readonly (keyof PlaceCapabilities)[] = [
  "read_only", "can_create", "can_rename", "can_trash", "can_copy_in",
  "can_copy_out", "knows_sizes", "knows_counts", "searchable", "has_thumbnails",
];

/** A row from its flags, in `FLAGS`' order. */
function row(...flags: boolean[]): PlaceCapabilities {
  return Object.fromEntries(FLAGS.map((flag, i) => [flag, flags[i]])) as unknown as PlaceCapabilities;
}

const T = true;
const F = false;

/** The table before the backend answers: the backend's own, as
 *  `tests/capabilities.json` pins it. */
export const BUILT_IN_CAPABILITIES: CapabilityTable = {
  //                     read_only create rename trash copy_in copy_out sizes counts search thumbnails
  folder: row(             F,       T,     T,     T,    T,      T,       T,    T,     T,     T),
  drive: row(              F,       T,     T,     T,    T,      T,       T,    F,     F,     T),
  share: row(              T,       F,     F,     F,    F,      F,       T,    F,     F,     F),
  sftp: row(               T,       F,     F,     F,    F,      F,       T,    F,     F,     F),
  "arbesk-account": row(   T,       T,     F,     F,    F,      F,       F,    F,     F,     T),
  "arbesk-collection": row(T,       F,     F,     F,    T,      F,       F,    F,     F,     T),
  "arbesk-asset": row(     T,       F,     F,     F,    F,      T,       F,    F,     F,     T),
  "arbesk-file": row(      T,       F,     F,     F,    F,      T,       F,    F,     F,     F),
};

let table: CapabilityTable = BUILT_IN_CAPABILITIES;

/** What `kind` allows. */
export function capabilitiesOf(kind: CapabilityRow): PlaceCapabilities {
  return table[kind];
}

/** Whether `answer` is a whole table: every row, every flag a boolean. */
export function isCapabilityTable(answer: unknown): answer is CapabilityTable {
  if (typeof answer !== "object" || answer === null) return false;
  const rows = answer as Record<string, unknown>;
  return ROWS.every((kind) => {
    const flags = rows[kind] as Record<string, unknown> | undefined;
    return typeof flags === "object" && flags !== null && FLAGS.every((flag) => typeof flags[flag] === "boolean");
  });
}

/** Take the backend's table; anything but a whole one keeps the table
 *  there is. */
export function setCapabilities(answer: unknown): void {
  if (isCapabilityTable(answer)) table = answer;
}

/** Back to the built-in table (tests). */
export function resetCapabilities(): void {
  table = BUILT_IN_CAPABILITIES;
}

/** Read the backend's table once, at startup; nothing waits for it, and a
 *  failure keeps the built-in table, which says the same. */
export async function loadCapabilities(fetch: () => Promise<unknown>): Promise<void> {
  setCapabilities(await fetch().catch(() => null));
}

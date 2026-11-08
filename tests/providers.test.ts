// The online account providers' table (src/providers.ts): one row per
// provider, and the account helpers every surface uses.
import { expect, test } from "vitest";
import {
  accountOf, fromDrive, PROVIDER_ORDER, PROVIDERS, providerOf, withAccount, withoutAccount, type Account, type ProviderId,
} from "../src/providers";

const ME: Account = { provider: "gdrive", id: "a1", email: "me@example.com", rootPath: "gdrive://a1/root~My%20Drive" };
const WORK: Account = { provider: "gdrive", id: "a2", email: "work@example.com", rootPath: "gdrive://a2/root~My%20Drive" };

test("every provider has a row and is listed once, in order", () => {
  const ids = Object.keys(PROVIDERS) as ProviderId[];
  expect([...PROVIDER_ORDER].sort()).toEqual([...ids].sort());
  expect(new Set(PROVIDER_ORDER).size).toBe(PROVIDER_ORDER.length);
  for (const id of ids) {
    expect(PROVIDERS[id].label).not.toBe("");
    expect(PROVIDERS[id].icon).toMatch(/^[a-z-]+$/);
  }
});

test.each([
  ["gdrive://a1/root~My%20Drive", "gdrive"],
  ["gdrive://a1/root~My%20Drive/f1~Docs", "gdrive"],
  ["/Users/me", null],
  ["share://abc", null],
  ["sftp://me@host/home", null],
  ["pseudo:network", null],
])("%s belongs to provider %s", (path, provider) => {
  expect(providerOf(path)).toBe(provider);
});

test("a path belongs to the account whose root shares its account key", () => {
  const accounts = [ME, WORK];
  expect(accountOf(ME.rootPath, accounts)).toBe(ME);
  expect(accountOf("gdrive://a2/root~My%20Drive/f1~Docs", accounts)).toBe(WORK);
  expect(accountOf("gdrive://a3/root~My%20Drive", accounts)).toBeUndefined();
  expect(accountOf("/Users/me", accounts)).toBeUndefined();
});

test("a sign-in replaces the account with its id in place, or adds it last", () => {
  const renamed = { ...ME, email: "new@example.com" };
  expect(withAccount([ME, WORK], renamed)).toEqual([renamed, WORK]);
  expect(withAccount([ME], WORK)).toEqual([ME, WORK]);
  expect(withoutAccount([ME, WORK], ME)).toEqual([WORK]);
  expect(withoutAccount([WORK], ME)).toEqual([WORK]);
});

test("a Google account from the backend becomes a Google Drive account", () => {
  expect(
    fromDrive({
      id: "a1",
      email: "me@example.com",
      name: "Me",
      root_id: "root",
      root_name: "My Drive",
      root_path: "gdrive://a1/root~My%20Drive",
      scopes: [],
      connected_at: 0,
    }),
  ).toEqual(ME);
});

// The online account providers' table (src/providers.ts): one row per
// provider, and the account helpers every surface uses.
import { expect, test } from "vitest";
import {
  accountOf, fromArbesk, fromDrive, PROVIDER_ORDER, PROVIDERS, providerOf, withAccount, withoutAccount, type Account, type ProviderId,
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
  ["arbesk://0x00000000000000000000000000000000000000ab", "arbesk"],
  ["arbesk://0x00000000000000000000000000000000000000ab/1~Set/a1~Box.gltf", "arbesk"],
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

test("Google Drive is listed before Arbesk", () => {
  expect(PROVIDER_ORDER).toEqual(["gdrive", "arbesk"]);
  expect(PROVIDERS.arbesk.label).toBe("Arbesk");
});

test("an Arbesk account is keyed by its wallet address", () => {
  const backend = { id: "x1", email: "me@example.com", address: "0x00000000000000000000000000000000000000ab", origin: "https://promptscad.com", chain_id: 84532, root_path: "arbesk://0x00000000000000000000000000000000000000ab" };
  const account = fromArbesk(backend);
  expect(account).toEqual({ provider: "arbesk", id: "x1", email: "me@example.com", rootPath: "arbesk://0x00000000000000000000000000000000000000ab" });
  expect(accountOf("arbesk://0x00000000000000000000000000000000000000ab/1~Set/a1~Box.gltf", [ME, account])).toBe(account);
  expect(accountOf("arbesk://0x00000000000000000000000000000000000000cd", [account])).toBeUndefined();
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

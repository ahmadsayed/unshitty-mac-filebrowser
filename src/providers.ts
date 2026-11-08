// Online accounts without the DOM: which providers File Browser can connect
// (one row each), the accounts the app knows in one shape whatever their
// provider, and which account a path belongs to. The Online Accounts dialog
// (src/online-accounts.ts), Network's rows and its Disconnect… all read this
// table, so a new provider is a row here, never a button or a branch at a
// call site. What a provider *does* (its sign-in dialogs) is wired by
// main.ts as a `Record<ProviderId, ProviderActions>`, so a missing one is a
// type error.

import type { ArbeskAccount, DriveAccount } from "./api";
import { arbeskAccountKey, cloudAccountKey, isArbeskPath, isCloudPath } from "./paths";

export type ProviderId = "gdrive" | "arbesk";

/** A connected account, whatever its provider. */
export interface Account {
  provider: ProviderId;
  /** The backend's account id. */
  id: string;
  /** The account's address, shown under the provider's name. */
  email: string;
  /** Where the account's files start: its entry in Network. */
  rootPath: string;
}

export interface Provider {
  /** The provider's name: the dialog's row and Network's entry. */
  label: string;
  /** A symbolic icon id from src/icons.ts, drawn by the app. */
  icon: string;
  /** Whether a path is one of this provider's. */
  owns(path: string): boolean;
  /** The account a path of this provider belongs to, as a key its root path shares. */
  accountKey(path: string): string;
}

export const PROVIDERS: Record<ProviderId, Provider> = {
  gdrive: { label: "Google Drive", icon: "cloud", owns: isCloudPath, accountKey: cloudAccountKey },
  arbesk: { label: "Arbesk", icon: "arbesk", owns: isArbeskPath, accountKey: arbeskAccountKey },
};

/** The order Online Accounts lists the providers in. */
export const PROVIDER_ORDER: ProviderId[] = ["gdrive", "arbesk"];

/** What a provider does, wired by main.ts to its own dialogs. */
export interface ProviderActions {
  /** Connect a new account (or one already connected, which keeps its row). */
  connect(): void;
  /** Sign the account in again after its sign-in expired or was revoked. */
  signIn(account: Account): void;
  /** Ask, then disconnect the account; its files are not touched. */
  disconnect(account: Account): void;
  /** Extra links on the account's dialog, such as Change Client…. */
  links(account: Account): { label: string; action: () => void }[];
}

/** A Google account as the backend reports it, in the shared shape. */
export function fromDrive(account: DriveAccount): Account {
  return { provider: "gdrive", id: account.id, email: account.email, rootPath: account.root_path };
}

/** An Arbesk account as the backend reports it, in the shared shape. */
export function fromArbesk(account: ArbeskAccount): Account {
  return { provider: "arbesk", id: account.id, email: account.email, rootPath: account.root_path };
}

/** The provider whose path this is, if any. */
export function providerOf(path: string): ProviderId | null {
  return PROVIDER_ORDER.find((id) => PROVIDERS[id].owns(path)) ?? null;
}

/** The account `path` belongs to: the one whose root shares its account key. */
export function accountOf(path: string, accounts: readonly Account[]): Account | undefined {
  const provider = providerOf(path);
  if (!provider) return undefined;
  const { accountKey } = PROVIDERS[provider];
  const key = accountKey(path);
  return accounts.find((account) => account.provider === provider && accountKey(account.rootPath) === key);
}

/** `accounts` with `account` in place of the one with its id, or added last. */
export function withAccount(accounts: readonly Account[], account: Account): Account[] {
  const known = accounts.some((item) => item.provider === account.provider && item.id === account.id);
  return known
    ? accounts.map((item) => (item.provider === account.provider && item.id === account.id ? account : item))
    : [...accounts, account];
}

/** `accounts` without `account`. */
export function withoutAccount(accounts: readonly Account[], account: Account): Account[] {
  return accounts.filter((item) => !(item.provider === account.provider && item.id === account.id));
}

// Online Accounts: the dialog that connects and manages accounts, shaped like
// GNOME Settings' Online Accounts panel ("Your Accounts" and "Connect an
// Account", boxed lists of rows with a chevron), and each account's own
// dialog (Sign In Again, Disconnect…, the provider's links). Built from
// src/providers.ts's table: no provider is named here.

import { sym } from "./icons";
import { PROVIDER_ORDER, PROVIDERS, type Account, type ProviderActions, type ProviderId } from "./providers";
import { dialogLinks, openDialog, type DialogHandle } from "./ui";

export interface OnlineAccountsHost {
  /** The connected accounts, in the order Network lists them. */
  accounts(): Account[];
  /** Whether the account's last request failed for want of a sign-in. */
  needsSignIn(account: Account): boolean;
  /** What each provider does: its own sign-in and disconnect dialogs. */
  actions: Record<ProviderId, ProviderActions>;
  /** Show the account's files. */
  open(account: Account): void;
}

/** One row of a boxed list: icon, title, optional subtitle and suffix, chevron. */
function actionRow(icon: string, title: string, subtitle: string, suffix: HTMLElement | null, onActivate: () => void): HTMLButtonElement {
  const row = document.createElement("button");
  row.className = "action-row";
  row.innerHTML = `${sym(icon, "action-row-icon")}<span class="action-row-text"><span class="action-row-title"></span></span>`;
  row.querySelector(".action-row-title")!.textContent = title;
  if (subtitle) {
    const sub = document.createElement("span");
    sub.className = "action-row-subtitle";
    sub.textContent = subtitle;
    row.querySelector(".action-row-text")!.appendChild(sub);
  }
  if (suffix) row.appendChild(suffix);
  row.insertAdjacentHTML("beforeend", sym("go-next", "action-row-chevron"));
  row.addEventListener("click", onActivate);
  return row;
}

/** A titled group holding a boxed list. */
function group(title: string, rows: HTMLElement[]): HTMLElement {
  const section = document.createElement("section");
  section.className = "preferences-group";
  const heading = document.createElement("h3");
  heading.className = "preferences-group-title";
  heading.textContent = title;
  const list = document.createElement("div");
  list.className = "boxed-list";
  list.setAttribute("role", "group");
  list.setAttribute("aria-label", title);
  list.append(...rows);
  section.append(heading, list);
  return section;
}

/** The "Sign in again" mark an account row carries, as Settings shows "Error". */
function signInMark(): HTMLElement {
  const mark = document.createElement("span");
  mark.className = "action-row-error";
  mark.innerHTML = `${sym("dialog-error")}<span>Sign in again</span>`;
  return mark;
}

/** The Online Accounts dialog: Your Accounts, then Connect an Account. */
export function showOnlineAccounts(host: OnlineAccountsHost): DialogHandle {
  const extra = document.createElement("div");
  extra.className = "online-accounts";
  const accounts = host.accounts();
  if (accounts.length) {
    const rows = accounts.map((account) =>
      actionRow(
        PROVIDERS[account.provider].icon,
        PROVIDERS[account.provider].label,
        account.email,
        host.needsSignIn(account) ? signInMark() : null,
        () => {
          dialog.close();
          showAccount(host, account);
        },
      ),
    );
    extra.appendChild(group("Your Accounts", rows));
  }
  const providers = PROVIDER_ORDER.map((id) =>
    actionRow(PROVIDERS[id].icon, PROVIDERS[id].label, "", null, () => {
      dialog.close();
      host.actions[id].connect();
    }),
  );
  extra.appendChild(group("Connect an Account", providers));

  const dialog = openDialog({
    heading: "Online Accounts",
    body: "Connect an account to browse its files in Network.",
    extra,
    className: "online-accounts-dialog",
    responses: [{ id: "close", label: "Close" }],
    closeResponse: "close",
  });
  return dialog;
}

/**
 * One account's dialog: whether it is connected, the provider's links, and
 * Disconnect…; Sign In Again too when its last request needed a sign-in.
 */
export function showAccount(host: OnlineAccountsHost, account: Account): DialogHandle {
  const provider = PROVIDERS[account.provider];
  const actions = host.actions[account.provider];
  const expired = host.needsSignIn(account);
  const extra = document.createElement("div");
  const status = document.createElement("p");
  status.className = "dialog-text account-status";
  status.textContent = expired
    ? "The sign-in expired or was revoked. Sign in again to keep browsing its files."
    : "Connected";
  extra.append(
    status,
    dialogLinks([
      {
        label: "Browse Files",
        action: () => {
          dialog.close();
          host.open(account);
        },
      },
      ...actions.links(account).map((link) => ({
        label: link.label,
        action: () => {
          dialog.close();
          link.action();
        },
      })),
    ]),
  );
  const dialog = openDialog({
    heading: provider.label,
    body: account.email,
    extra,
    responses: [
      { id: "close", label: "Close" },
      { id: "disconnect", label: "Disconnect…", style: "destructive" },
      ...(expired ? [{ id: "sign-in", label: "Sign In Again", style: "suggested" as const }] : []),
    ],
    defaultResponse: expired ? "sign-in" : "close",
    closeResponse: "close",
    onResponse: (id) => {
      // The provider's own dialog takes over once this one has closed.
      if (id === "disconnect") setTimeout(() => actions.disconnect(account));
      if (id === "sign-in") setTimeout(() => actions.signIn(account));
    },
  });
  return dialog;
}

// Check for Updates: the dialogs from the main menu item to the restart.
// The backend decides what this copy may do (docs/superpowers/specs/
// 2026-10-29-updates-design.md); nothing here talks to the network itself.

import {
  cancelUpdate,
  checkUpdate,
  installUpdate,
  openExternal,
  restartApp,
  type UpdateFailure,
  type UpdateInfo,
  type UpdateProgress,
} from "./api";
import { formatSize } from "./format";
import { hasRunningOperations, onOperationsChange } from "./operations";
import { dialogLinks, openDialog, type DialogResponse } from "./ui";

const HEADING = "Check for Updates";
const CLOSE: DialogResponse = { id: "close", label: "Close" };
const RELEASE_PAGE: DialogResponse = { id: "release", label: "Open Release Page" };
const TRY_AGAIN: DialogResponse = { id: "retry", label: "Try Again", style: "suggested" };

/** A rejected command's error as an UpdateFailure, whatever shape it came in. */
function asFailure(error: unknown): UpdateFailure {
  if (error && typeof error === "object" && "kind" in error) return error as UpdateFailure;
  return { kind: "failed", detail: String(error) };
}

function textBlock(...lines: string[]): HTMLElement {
  const box = document.createElement("div");
  for (const line of lines) {
    const p = document.createElement("p");
    p.textContent = line;
    box.appendChild(p);
  }
  return box;
}

/** A message dialog whose buttons each lead somewhere (or just close). */
function showMessage(
  heading: string,
  body: string,
  responses: DialogResponse[],
  actions: Record<string, () => void> = {},
  extra?: HTMLElement
): void {
  openDialog({
    heading,
    body,
    extra,
    responses,
    closeResponse: "close",
    onResponse: (id) => actions[id]?.(),
  });
}

/** Entry point: the main menu's "Check for Updates…". */
export function checkForUpdates(): void {
  void runCheck();
}

async function runCheck(): Promise<void> {
  let cancelled = false;
  const extra = textBlock("Checking for updates…");
  extra.classList.add("update-checking");
  const dialog = openDialog({
    heading: HEADING,
    extra,
    responses: [{ id: "cancel", label: "Cancel" }],
    onResponse: () => {
      cancelled = true;
    },
  });
  let info: UpdateInfo;
  try {
    info = await checkUpdate();
  } catch (error) {
    if (cancelled) return;
    dialog.close();
    return showCheckFailed(asFailure(error));
  }
  if (cancelled) return;
  dialog.close();
  showResult(info);
}

function showCheckFailed(failure: UpdateFailure): void {
  const reason = failure.kind === "offline" ? "No internet connection." : failure.detail;
  showMessage(HEADING, "Could not check for updates.", [CLOSE, TRY_AGAIN], { retry: checkForUpdates }, textBlock(reason));
}

/** What the check found, worded for how this copy was installed. */
function showResult(info: UpdateInfo): void {
  if (!info.latest) {
    return showMessage(HEADING, `File Browser ${info.current} is the latest version.`, [CLOSE]);
  }
  const available = `File Browser ${info.latest} is available. You have ${info.current}.`;
  const openRelease = () => void openExternal(info.release_url);
  if (info.kind === "linux_package") {
    return showMessage("Update Available", `${available} Update it with your system's package manager.`, [CLOSE, RELEASE_PAGE], {
      release: openRelease,
    });
  }
  if (info.kind === "dev_build") {
    return showMessage("Update Available", `${available} Updates are installed only in release builds.`, [CLOSE, RELEASE_PAGE], {
      release: openRelease,
    });
  }
  openDialog({
    heading: "Update Available",
    body: available,
    extra: dialogLinks([{ label: "What's New", action: openRelease }]),
    responses: [
      { id: "later", label: "Not Now" },
      { id: "install", label: "Download and Install", style: "suggested" },
    ],
    defaultResponse: "later",
    closeResponse: "later",
    onResponse: (id) => {
      if (id === "install") void runInstall(info);
    },
  });
}

/** "Downloading… 23.4 MB of 91.4 MB", or without the total when unknown. */
function progressText(progress: UpdateProgress): string {
  if (progress.phase === "installing") return "Installing…";
  const done = formatSize(progress.downloaded);
  return progress.total === null ? `Downloading… ${done}` : `Downloading… ${done} of ${formatSize(progress.total)}`;
}

async function runInstall(info: UpdateInfo): Promise<void> {
  let cancelled = false;
  const text = textBlock("Downloading…");
  const bar = document.createElement("div");
  bar.className = "operation-bar update-bar";
  const fill = document.createElement("div");
  fill.className = "operation-bar-fill";
  fill.style.width = "0%";
  bar.appendChild(fill);
  const extra = document.createElement("div");
  extra.append(text, bar);
  const dialog = openDialog({
    heading: `Updating to File Browser ${info.latest}`,
    extra,
    responses: [{ id: "cancel", label: "Cancel" }],
    onResponse: () => {
      cancelled = true;
      void cancelUpdate();
    },
  });
  const onProgress = (progress: UpdateProgress) => {
    text.firstElementChild!.textContent = progressText(progress);
    if (progress.total) fill.style.width = `${Math.min(100, (progress.downloaded / progress.total) * 100)}%`;
    // Installing can't be stopped: the backend no longer listens for Cancel.
    if (progress.phase === "installing") dialog.setResponseEnabled("cancel", false);
  };
  try {
    await installUpdate(onProgress);
  } catch (error) {
    if (cancelled) return;
    dialog.close();
    return showInstallFailed(info, asFailure(error));
  }
  if (cancelled) return;
  dialog.close();
  showInstalled(info);
}

function showInstallFailed(info: UpdateInfo, failure: UpdateFailure): void {
  const openRelease = () => void openExternal(info.release_url);
  if (failure.kind === "unverified") {
    return showMessage("Update Failed", "The update could not be verified and was not installed.", [CLOSE]);
  }
  if (failure.kind === "no_permission") {
    return showMessage(
      "Update Failed",
      "File Browser can't replace itself in this folder. Download the new version from the release page.",
      [CLOSE, RELEASE_PAGE],
      { release: openRelease }
    );
  }
  showMessage("Update Failed", "The download failed.", [CLOSE, TRY_AGAIN], { retry: () => void runInstall(info) }, textBlock(failure.detail));
}

/** Installed: restart now, or keep working and get it next time. Restarting
 * waits for running copies and moves, which would otherwise be cut off. */
function showInstalled(info: UpdateInfo): void {
  const hint = textBlock("Waiting for operations to finish…");
  hint.className = "dim";
  let unsubscribe = () => {};
  const dialog = openDialog({
    heading: `File Browser ${info.latest} is installed.`,
    body: "Restart to use it.",
    extra: hint,
    responses: [
      { id: "later", label: "Later" },
      { id: "restart", label: "Restart Now", style: "suggested" },
    ],
    defaultResponse: "later",
    closeResponse: "later",
    onResponse: (id) => {
      unsubscribe();
      if (id === "restart") void restartApp();
    },
  });
  const update = () => {
    const busy = hasRunningOperations();
    dialog.setResponseEnabled("restart", !busy);
    hint.hidden = !busy;
    hint.firstElementChild!.textContent = busy ? "Waiting for operations to finish…" : "";
  };
  unsubscribe = onOperationsChange(update);
  update();
}

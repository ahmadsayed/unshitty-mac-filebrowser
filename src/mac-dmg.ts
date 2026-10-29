// DMG inspection always ejects before returning. No mounted resource belongs
// to a dialog, and no application is opened until the user explicitly asks.
import { inspectDmg, installDmg, openFile, type DmgInspection, type DmgInstallMode, type Entry } from "./api";
import { openDialog, showToast, type MenuItem } from "./ui";

export function macDmgItems(entry: Entry | null, enabled: boolean): MenuItem[] {
  if (!enabled || !entry || entry.is_dir || entry.is_symlink || !entry.path.startsWith("/")) return [];
  if (entry.path.split("/").includes(".versions")) return [];
  return /\.dmg$/i.test(entry.name) ? [{ label: "Install Application…", action: () => void prepare(entry) }] : [];
}

async function prepare(entry: Entry): Promise<void> {
  let cancelled = false;
  const loading = openDialog({
    heading: "Reading Disk Image…",
    body: "Looking for applications. Nothing in the disk image will be run.",
    responses: [{ id: "cancel", label: "Cancel" }],
    defaultResponse: "cancel", closeResponse: "cancel",
    onResponse: () => { cancelled = true; },
  });
  try {
    const inspection = await inspectDmg(entry.path);
    loading.close();
    if (!cancelled) installDialog(entry, inspection);
  } catch (err) {
    loading.close();
    if (!cancelled) showToast(`Could not read disk image: ${String(err)}`);
  }
}

function conflictChoice(name: string): Promise<DmgInstallMode | null> {
  return new Promise((resolve) => openDialog({
    heading: `Replace application “${name}”?`,
    body: "Another item with the same name already exists. Quit that application before replacing it. Replace moves the existing application to the Trash; Keep Both installs a separate copy.",
    responses: [{ id: "skip", label: "Skip" }, { id: "keep-both", label: "Keep Both" }, { id: "replace", label: "Replace", style: "destructive" }],
    defaultResponse: "keep-both", closeResponse: "skip",
    onResponse: (id) => resolve(id === "skip" ? null : id as DmgInstallMode),
  }));
}

function installDialog(entry: Entry, info: DmgInspection): void {
  const extra = document.createElement("div");
  extra.className = "compress-fields";
  extra.innerHTML = `<label>Application<select class="entry" id="dmg-application"></select></label>` +
    `<label>Install for<select class="entry" id="dmg-destination"><option value="user">My Applications</option><option value="system">All Users</option></select></label>` +
    `<div class="field-message" role="status"></div>`;
  const apps = extra.querySelector<HTMLSelectElement>("#dmg-application")!;
  const destination = extra.querySelector<HTMLSelectElement>("#dmg-destination")!;
  const message = extra.querySelector<HTMLElement>(".field-message")!;
  for (const app of info.apps) {
    const option = document.createElement("option");
    option.value = app.id;
    option.textContent = `${app.name} (${app.fileName})`;
    apps.appendChild(option);
  }
  message.textContent = info.warning ?? "";
  let busy = false;
  const dialog = openDialog({
    heading: "Install Application?",
    body: `Choose an application from “${entry.name}”. Only install software you trust: applications can access your files. The original disk image and macOS security checks will be preserved. Package installers are not run by this action.`,
    extra,
    responses: [{ id: "cancel", label: "Cancel" }, { id: "install", label: "Install", style: "suggested" }],
    defaultResponse: "cancel", closeResponse: "cancel",
    onResponse: async (id) => {
      if (busy) return false;
      if (id !== "install") return;
      busy = true;
      apps.disabled = destination.disabled = true;
      dialog.setResponseEnabled("install", false);
      dialog.setResponseEnabled("cancel", false);
      message.classList.remove("error");
      message.textContent = "Installing…";
      try {
        const target = destination.value as "user" | "system";
        let result = await installDmg(entry.path, info.fingerprint, apps.value, target, "install");
        if (result.kind === "conflict") {
          const choice = await conflictChoice(result.name);
          if (!choice) return;
          result = await installDmg(entry.path, info.fingerprint, apps.value, target, choice);
        }
        if (result.kind !== "installed") throw "Another item appeared at the destination. Try again.";
        const installed = result;
        showToast(`Installed “${installed.name}”.${installed.warning ? ` ${installed.warning}` : ""}`, {
          button: { label: "Open", onClick: () => void openFile(installed.path).catch((err) => showToast(`Could not open application: ${String(err)}`)) },
        });
      } catch (err) {
        message.classList.add("error");
        message.textContent = String(err);
        return false;
      } finally {
        busy = false;
        apps.disabled = destination.disabled = false;
        dialog.setResponseEnabled("install", true);
        dialog.setResponseEnabled("cancel", true);
      }
    },
  });
  dialog.setResponseEnabled("install", info.apps.length > 0);
}

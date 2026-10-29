// Viewing a menu never changes permissions or executes a program.
import { installAppImage, runShellScript, type Entry } from "./api";
import { openDialog, showToast, type MenuItem } from "./ui";

export function linuxProgramItems(entry: Entry | null, enabled: boolean): MenuItem[] {
  if (!enabled || !entry || entry.is_dir || entry.is_symlink || !entry.path.startsWith("/")) return [];
  if (entry.path.split("/").includes(".versions")) return [];
  if (/\.appimage$/i.test(entry.name)) return [{ label: "Install Application…", action: () => installDialog(entry) }];
  if (/\.(sh|bash)$/i.test(entry.name)) return [{ label: "Run in Terminal…", action: () => runDialog(entry) }];
  return [];
}

function applicationNameError(name: string): string {
  if (!name.trim()) return "Enter an application name.";
  if (/[\\/\u0000-\u001f\u007f]/.test(name)) return "Application names cannot contain “/”, “\\” or control characters.";
  if ([...name.trim()].length > 100) return "Application name is too long.";
  return "";
}

function installDialog(entry: Entry): void {
  const extra = document.createElement("div");
  extra.innerHTML = `<div class="entry-row"><label for="appimage-name">Application name</label>` +
    `<input id="appimage-name" type="text" spellcheck="false" autocomplete="off" /></div>` +
    `<div class="field-message" role="status"></div>`;
  const input = extra.querySelector<HTMLInputElement>("input")!;
  const message = extra.querySelector<HTMLElement>(".field-message")!;
  input.value = entry.name.replace(/\.appimage$/i, "");
  let busy = false;
  const validate = () => {
    const error = applicationNameError(input.value);
    message.textContent = error;
    message.classList.toggle("error", !!error);
    dialog.setResponseEnabled("install", !busy && !error);
    return !error;
  };
  const dialog = openDialog({
    heading: "Install Application?",
    body: `“${entry.name}” will be copied to your application folder and added to application search with a generic icon. The download stays where it is. Only install applications you trust: they can access your files when opened.`,
    extra,
    responses: [{ id: "cancel", label: "Cancel" }, { id: "install", label: "Install", style: "suggested" }],
    defaultResponse: "cancel",
    closeResponse: "cancel",
    onResponse: async (id) => {
      if (busy) return false;
      if (id !== "install") return;
      if (!validate()) return false;
      busy = true;
      input.disabled = true;
      dialog.setResponseEnabled("cancel", false);
      dialog.setResponseEnabled("install", false);
      message.textContent = "Installing…";
      try {
        const installed = await installAppImage(entry.path, input.value.trim());
        showToast(`Installed “${installed.name}”. Find it in application search.`);
        return;
      } catch (err) {
        message.classList.add("error");
        message.textContent = String(err);
        return false;
      } finally {
        busy = false;
        input.disabled = false;
        dialog.setResponseEnabled("cancel", true);
        dialog.setResponseEnabled("install", true);
      }
    },
  });
  input.addEventListener("input", validate);
  validate();
}

function runDialog(entry: Entry): void {
  const extra = document.createElement("div");
  extra.className = "field-message error";
  extra.setAttribute("role", "status");
  let busy = false;
  const dialog = openDialog({
    heading: "Run Script?",
    body: `“${entry.name}” will run using Bash in a terminal, from its containing folder. Only run scripts you trust: they can change or delete your files. File permissions will not be changed.`,
    extra,
    responses: [{ id: "cancel", label: "Cancel" }, { id: "run", label: "Run in Terminal", style: "suggested" }],
    defaultResponse: "cancel",
    closeResponse: "cancel",
    onResponse: async (id) => {
      if (busy) return false;
      if (id !== "run") return;
      busy = true;
      dialog.setResponseEnabled("run", false);
      dialog.setResponseEnabled("cancel", false);
      try {
        await runShellScript(entry.path);
        showToast("Terminal requested. Check the script’s output there.");
      } catch (err) {
        extra.textContent = String(err);
        return false;
      } finally {
        busy = false;
        dialog.setResponseEnabled("run", true);
        dialog.setResponseEnabled("cancel", true);
      }
    },
  });
}

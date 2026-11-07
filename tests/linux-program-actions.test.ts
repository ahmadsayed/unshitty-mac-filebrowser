import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, $$, boot, chooseMenu, closeOverlays, dialogHeading, goTo, hit, key, menuLabels, mouse, respond, settle, toasts, type } from "./app";

const HOME = "/Users/me";
async function rightClick(name: string): Promise<void> {
  await mouse(hit(name), "mousedown", { button: 2 });
  await mouse(hit(name), "contextmenu", { button: 2 });
}

beforeAll(async () => {
  Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (X11; Linux x86_64)", configurable: true });
  await boot();
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/Demo.AppImage`, overwrite: false });
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/run.sh`, overwrite: false });
  await goTo(HOME);
});

test("an AppImage installs only after confirmation and preserves the source", async () => {
  await rightClick("Demo.AppImage");
  expect(menuLabels()).toContain("Install Application…");
  await chooseMenu("Install Application…");
  expect(dialogHeading()).toBe("Install Application?");
  await key("Enter", { target: $("#appimage-name") });
  expect(dialogHeading()).toBeUndefined();
  expect(await invoke("path_exists", { path: `${HOME}/.local/share/applications/demo.desktop` })).toBe(false);
  await rightClick("Demo.AppImage");
  await chooseMenu("Install Application…");
  await type($<HTMLInputElement>("#appimage-name"), "Demo");
  await respond("Install");
  expect(toasts().join(" ")).toContain("Installed “Demo”");
  expect(await invoke("path_exists", { path: `${HOME}/Demo.AppImage` })).toBe(true);
});

test("a Bash script requires confirmation and Enter defaults to Cancel", async () => {
  await goTo(HOME);
  await rightClick("run.sh");
  expect(menuLabels()).toContain("Run in Terminal…");
  expect(menuLabels()).not.toContain("Install Application…");
  await chooseMenu("Run in Terminal…");
  expect(dialogHeading()).toBe("Run Script?");
  await key("Enter");
  expect(dialogHeading()).toBeUndefined();
  await rightClick("run.sh");
  await chooseMenu("Run in Terminal…");
  await respond("Run in Terminal");
  expect(toasts().join(" ")).toContain("Terminal requested");
});

test("an install conflict stays inline and a different name can be used", async () => {
  await goTo(HOME);
  await rightClick("Demo.AppImage");
  await chooseMenu("Install Application…");
  await respond("Install");
  expect(dialogHeading()).toBe("Install Application?");
  expect($(".field-message").textContent).toContain("already installed");
  await type($<HTMLInputElement>("#appimage-name"), "Bad/Name");
  expect($(".field-message").textContent).toContain("cannot contain");
  await type($<HTMLInputElement>("#appimage-name"), "Demo Two");
  await respond("Install");
  expect(dialogHeading()).toBeUndefined();
  expect(toasts().join(" ")).toContain("Installed “Demo Two”");
});

test("a script that vanishes after opening its menu is not reported as launched", async () => {
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/gone.sh`, overwrite: false });
  await goTo(HOME);
  await key("F5");
  await rightClick("gone.sh");
  await chooseMenu("Run in Terminal…");
  await invoke("trash_entries", { paths: [`${HOME}/gone.sh`] });
  await respond("Run in Terminal");
  expect(dialogHeading()).toBe("Run Script?");
  expect($(".dialog-extra").textContent).toContain("Choose a local Bash script");
  await respond("Cancel");
});

type MockCalls = { count(cmd: string): number; hold(cmd: string): void; release(cmd: string): void; failNext(cmd: string, error: string): void; reset(): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
const button = (label: string) =>
  $$(".dialog-layer .dialog-responses button").find((b) => b.textContent?.trim() === label) as HTMLButtonElement;

test("an install in progress ignores a second Install, Enter and Escape", async () => {
  calls().reset();
  await goTo(HOME);
  await rightClick("Demo.AppImage");
  await chooseMenu("Install Application…");
  const input = $<HTMLInputElement>("#appimage-name");
  await type(input, "Demo Flight");
  calls().hold("install_appimage");
  await respond("Install");
  expect($(".field-message").textContent).toBe("Installing…");
  expect(button("Install").disabled).toBe(true);
  expect(button("Cancel").disabled).toBe(true);
  expect(input.disabled).toBe(true);
  await respond("Install");
  await key("Enter", { target: input });
  await key("Escape", { target: input });
  expect(dialogHeading()).toBe("Install Application?");
  calls().release("install_appimage");
  await settle(20);
  expect(calls().count("install_appimage")).toBe(1);
  expect(dialogHeading()).toBeUndefined();
  expect(toasts().join(" ")).toContain("Installed “Demo Flight”");
});

test("a failed install stays in the dialog, word for word, with everything enabled", async () => {
  calls().reset();
  await goTo(HOME);
  await rightClick("Demo.AppImage");
  await chooseMenu("Install Application…");
  const input = $<HTMLInputElement>("#appimage-name");
  await type(input, "Demo Fail");
  calls().failNext("install_appimage", "Failed to copy: Read-only file system (os error 30)");
  await respond("Install");
  expect(dialogHeading()).toBe("Install Application?");
  expect($(".field-message").textContent).toBe("Failed to copy: Read-only file system (os error 30)");
  expect($(".field-message").classList.contains("error")).toBe(true);
  expect(button("Install").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  expect(input.disabled).toBe(false);
  await respond("Cancel");
});

test("a script run in progress ignores a second Run, Enter and Escape", async () => {
  calls().reset();
  await goTo(HOME);
  await rightClick("run.sh");
  await chooseMenu("Run in Terminal…");
  calls().hold("run_shell_script");
  await respond("Run in Terminal");
  expect(button("Run in Terminal").disabled).toBe(true);
  expect(button("Cancel").disabled).toBe(true);
  await respond("Run in Terminal");
  await key("Enter");
  await key("Escape");
  expect(dialogHeading()).toBe("Run Script?");
  calls().release("run_shell_script");
  await settle(20);
  expect(calls().count("run_shell_script")).toBe(1);
  expect(dialogHeading()).toBeUndefined();
  expect(toasts().join(" ")).toContain("Terminal requested");
});

test("a failed script run stays in the dialog, word for word, with everything enabled", async () => {
  calls().reset();
  await goTo(HOME);
  await rightClick("run.sh");
  await chooseMenu("Run in Terminal…");
  calls().failNext("run_shell_script", "No terminal found: install GNOME Terminal (os error 2)");
  await respond("Run in Terminal");
  expect(dialogHeading()).toBe("Run Script?");
  expect($(".dialog-extra").textContent).toBe("No terminal found: install GNOME Terminal (os error 2)");
  expect(button("Run in Terminal").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  await respond("Cancel");
});

test("program actions are hidden on folders, multiple selections, Trash and macOS", async () => {
  await goTo(HOME);
  await rightClick("Documents");
  expect(menuLabels()).not.toContain("Install Application…");
  expect(menuLabels()).not.toContain("Run in Terminal…");
  await closeOverlays();
  await key("a", { ctrl: true });
  await rightClick("Demo.AppImage");
  expect(menuLabels()).not.toContain("Install Application…");
  await closeOverlays();
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/.Trash/trashed.sh`, overwrite: false });
  await goTo(`${HOME}/.Trash`);
  await rightClick("trashed.sh");
  expect(menuLabels()).not.toContain("Run in Terminal…");
  await closeOverlays();
  Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (Macintosh)", configurable: true });
  await goTo(HOME);
  await rightClick("Demo.AppImage");
  expect(menuLabels()).not.toContain("Install Application…");
  await closeOverlays();
});

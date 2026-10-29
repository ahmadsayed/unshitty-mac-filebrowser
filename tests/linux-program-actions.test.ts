import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, boot, chooseMenu, closeOverlays, dialogHeading, goTo, hit, key, menuLabels, mouse, respond, toasts, type } from "./app";

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

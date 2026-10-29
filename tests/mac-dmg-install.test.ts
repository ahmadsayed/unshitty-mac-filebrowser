import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, boot, chooseMenu, closeOverlays, dialogHeading, goTo, hit, key, menuLabels, mouse, respond, toasts } from "./app";

const HOME = "/Users/me";
async function rightClick(name: string): Promise<void> {
  await mouse(hit(name), "mousedown", { button: 2 });
  await mouse(hit(name), "contextmenu", { button: 2 });
}
beforeAll(async () => {
  await boot();
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/Sample.dmg`, overwrite: false });
  await goTo(HOME);
});

test("a DMG offers an app chooser and installation defaults to Cancel", async () => {
  await rightClick("Sample.dmg");
  expect(menuLabels()).toContain("Install Application…");
  await chooseMenu("Install Application…");
  expect(dialogHeading()).toBe("Install Application?");
  expect($<HTMLSelectElement>("#dmg-destination").value).toBe("user");
  await key("Enter");
  expect(dialogHeading()).toBeUndefined();
  expect(await invoke("path_exists", { path: `${HOME}/Applications/Sample.app` })).toBe(false);
  await rightClick("Sample.dmg");
  await chooseMenu("Install Application…");
  await respond("Install");
  expect(toasts().join(" ")).toContain("Installed “Sample”");
  expect(await invoke("path_exists", { path: `${HOME}/Sample.dmg` })).toBe(true);
});

test("an existing application asks and Enter chooses Keep Both", async () => {
  await rightClick("Sample.dmg");
  await chooseMenu("Install Application…");
  await respond("Install");
  expect(dialogHeading()).toBe("Replace application “Sample”?");
  await key("Enter");
  expect(dialogHeading()).toBeUndefined();
  expect(await invoke("path_exists", { path: `${HOME}/Applications/Sample.app` })).toBe(true);
  expect(await invoke("path_exists", { path: `${HOME}/Applications/Sample (copy).app` })).toBe(true);
});

test("multiple apps and the system destination are selectable", async () => {
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/Multiple.dmg`, overwrite: false });
  await goTo(HOME);
  await key("F5");
  await rightClick("Multiple.dmg");
  await chooseMenu("Install Application…");
  const apps = $<HTMLSelectElement>("#dmg-application");
  expect(apps.options.length).toBe(2);
  apps.value = "Second.app";
  $<HTMLSelectElement>("#dmg-destination").value = "system";
  await respond("Install");
  expect(await invoke("path_exists", { path: "/Applications/Second.app" })).toBe(true);
});

test("Skip leaves the existing app alone and Replace sends it to Trash", async () => {
  await goTo(HOME);
  await rightClick("Sample.dmg");
  await chooseMenu("Install Application…");
  await respond("Install");
  await respond("Skip");
  expect(await invoke("path_exists", { path: `${HOME}/Applications/Sample.app` })).toBe(true);
  await rightClick("Sample.dmg");
  await chooseMenu("Install Application…");
  await respond("Install");
  await respond("Replace");
  expect(await invoke("path_exists", { path: `${HOME}/.Trash/Sample.app` })).toBe(true);
});

test("package-only images show an error, and Trash files have no install action", async () => {
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/Packages.dmg`, overwrite: false });
  await goTo(HOME);
  await key("F5");
  await rightClick("Packages.dmg");
  await chooseMenu("Install Application…");
  expect(dialogHeading()).toBeUndefined();
  expect(toasts().join(" ")).toContain("Package installers are not supported");
  await invoke("copy_entry", { from: `${HOME}/Desktop/todo.txt`, to: `${HOME}/.Trash/Trash.dmg`, overwrite: false });
  await goTo(`${HOME}/.Trash`);
  await rightClick("Trash.dmg");
  expect(menuLabels()).not.toContain("Install Application…");
  await closeOverlays();
});

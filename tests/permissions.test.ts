// Properties → Permissions: choose the read/write/execute bits and apply them.
import { beforeAll, expect, test } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import { $, boot, chooseMenu, click, dialogHeading, goTo, item, key, mouse, respond, settle } from "./app";

const HOME = "/Users/me";

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

test("Permissions… changes the owner write bit", async () => {
  await goTo(`${HOME}/Desktop`);
  await rightClick(item("todo.txt"));
  await chooseMenu("Properties");
  await settle(20);
  expect(dialogHeading()).toBe("Properties");

  await respond("Permissions…");
  await settle(10);
  expect(dialogHeading()).toBe("Permissions");

  const ownerWrite = $<HTMLInputElement>('input[data-bit="128"]');
  expect(ownerWrite.checked).toBe(true);
  await click(ownerWrite);
  expect(ownerWrite.checked).toBe(false);

  await respond("Change");
  await settle(30);
  expect(dialogHeading()).toBe("Properties");
  const props = await invoke<Record<string, unknown>>("file_properties", {
    path: `${HOME}/Desktop/todo.txt`,
    showHidden: false,
  });
  expect(props).toMatchObject({ octal: "444", mode: "-r--r--r--" });
});

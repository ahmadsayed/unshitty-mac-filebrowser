// Open With: list installed applications and launch the chosen one.
import { beforeAll, expect, test } from "vitest";
import { $, $$, boot, chooseMenu, click, dialogHeading, goTo, item, key, mouse, settle } from "./app";

const HOME = "/Users/me";

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

test("Open With… lists applications and closes after choosing one", async () => {
  await goTo(`${HOME}/Documents`);
  await rightClick(item("Notes.md"));
  await chooseMenu("Open With…");
  await settle(20);
  expect(dialogHeading()).toBe("Open “Notes.md” With");
  expect($$(".app-choice").map((button) => button.textContent)).toEqual(
    expect.arrayContaining(["TextEdit", "Preview"])
  );

  const textEdit = $$(".app-choice").find((button) => button.textContent === "TextEdit")!;
  await click(textEdit);
  await settle(20);
  expect(document.querySelector(".dialog-layer")).toBeNull();
});

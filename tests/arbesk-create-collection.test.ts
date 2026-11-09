// Arbesk collections: New Collection… in the account, its name dialog, a
// collection that already exists, and the sign-in it needs. The account is
// connected through the mock before the app boots.
import { beforeAll, expect, test, vi } from "vitest";
import { invoke, ARBESK_MOCK_ADDRESS } from "../dev/mock-tauri/core";
import {
  $, $$, boot, chooseMenu, click, closeOverlays, dialogHeading, installFakeCanvas, menuLabels, names, open, respond, rightClick, settle, toasts, type,
} from "./app";

const arbeskMock = () => (globalThis as unknown as { __mockArbesk: { signedIn: boolean } }).__mockArbesk;

beforeAll(async () => {
  installFakeCanvas();
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  await boot();
});

async function openArbesk(): Promise<void> {
  await click($$(".sidebar-row").find((row) => row.textContent?.trim() === "Network")!);
  await settle(20);
  await open("Arbesk");
  await settle(20);
}

async function backgroundMenu(): Promise<void> {
  await rightClick($("#view"));
  await settle(5);
}

async function startNewCollection(): Promise<void> {
  await backgroundMenu();
  await chooseMenu("New Collection…");
  await settle(10);
}

/** Click `label` in the topmost dialog: a sign-in prompt sits over the name dialog. */
async function respondTop(label: string): Promise<void> {
  const layers = $$(".dialog-layer");
  const button = [...layers[layers.length - 1].querySelectorAll(".dialog-responses button")].find(
    (candidate) => candidate.textContent?.trim() === label,
  )!;
  await click(button);
  await settle(15);
}

test("New Collection… is in the account's menus, and not inside a collection", async () => {
  await openArbesk();
  await backgroundMenu();
  expect(menuLabels()[0]).toBe("New Collection…");
  await closeOverlays();

  await open("Shapes");
  await settle(20);
  await backgroundMenu();
  expect(menuLabels()).not.toContain("New Collection…");
  await closeOverlays();
  await openArbesk();
});

test("creating a collection asks for its name and lists it", async () => {
  await startNewCollection();
  expect(dialogHeading()).toBe("New Collection");
  expect($(".dialog-layer .dialog-text").textContent).toBe("Choose a name for the new collection.");
  const create = $$(".dialog-layer .dialog-responses button").find((b) => b.textContent === "Create")!;
  expect(create.hasAttribute("disabled")).toBe(true);

  await type($<HTMLInputElement>("#new-collection-name"), "My Set");
  expect(create.hasAttribute("disabled")).toBe(false);
  await respond("Create");
  await vi.waitFor(() => expect(names()).toContain("My Set"), { timeout: 2000 });
  expect(toasts()).toContain("Created “My Set”");

  await open("My Set");
  await settle(20);
  expect(names()).toEqual([]);
  expect($$(".crumb").map((el) => el.textContent?.trim())).toEqual(["Arbesk", "My Set"]);
  await openArbesk();
});

test("a name already minted stays in the dialog with the reason", async () => {
  await startNewCollection();
  await type($<HTMLInputElement>("#new-collection-name"), "My Set");
  await respond("Create");
  await settle(20);
  expect(dialogHeading()).toBe("New Collection");
  expect($(".dialog-layer .field-message").textContent).toBe("A collection named “My Set” already exists.");
  await respond("Cancel");
  await settle(20);
  expect(dialogHeading()).toBeUndefined();
  expect(names().filter((name) => name === "My Set")).toHaveLength(1);
});

test("a creation with no session signs in first; Cancel creates nothing", async () => {
  arbeskMock().signedIn = false;
  await startNewCollection();
  await type($<HTMLInputElement>("#new-collection-name"), "Second Set");
  await respond("Create");
  expect(dialogHeading()).toBe("Sign In to Arbesk");
  expect($<HTMLInputElement>("#arbesk-email").value).toBe("me@example.com");
  await respond("Open Browser");
  await vi.waitFor(() => expect(names()).toContain("Second Set"), { timeout: 2000 });
  expect(toasts()).toContain("Created “Second Set”");

  arbeskMock().signedIn = false;
  await startNewCollection();
  await type($<HTMLInputElement>("#new-collection-name"), "Third Set");
  await respond("Create");
  expect(dialogHeading()).toBe("Sign In to Arbesk");
  await respondTop("Cancel");
  await settle(20);
  expect(dialogHeading()).toBeUndefined();
  expect(names()).not.toContain("Third Set");
  arbeskMock().signedIn = true;
});

test("New Collection is reachable from the path bar's menu too", async () => {
  await openArbesk();
  await click($("#btn-location-menu"));
  await settle(5);
  expect(menuLabels()).toContain("New Collection…");
  await closeOverlays();
});

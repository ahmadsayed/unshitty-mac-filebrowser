// Arbesk sharing: Share Collection… on a collection, by email, as a viewer
// or an editor. The account is connected through the mock before the app boots.
import { beforeAll, expect, test, vi } from "vitest";
import { invoke } from "../dev/mock-tauri/core";
import {
  $, $$, boot, chooseMenu, click, closeOverlays, dialogHeading, installFakeCanvas, key, menuLabels, open, respond, rightClick, settle, toasts, type,
} from "./app";

const arbeskMock = () =>
  (globalThis as unknown as { __mockArbesk: { signedIn: boolean; expireOnce(): void } }).__mockArbesk;
const row = (name: string) => $$("#view .item").find((el) => el.querySelector(".name")?.textContent === name)!;

beforeAll(async () => {
  installFakeCanvas();
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
  await boot();
  await key("1", { mod: true }); // the list view, whose rows take an item menu
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

async function openShareDialog(): Promise<void> {
  await backgroundMenu();
  await chooseMenu("Share Collection…");
  await settle(10);
}

/** Click `label` in the topmost dialog (a sign-in prompt can sit over another). */
async function respondTop(label: string): Promise<void> {
  const layers = $$(".dialog-layer");
  const button = [...layers[layers.length - 1].querySelectorAll(".dialog-responses button")].find(
    (candidate) => candidate.textContent?.trim() === label,
  )!;
  await click(button);
  await settle(15);
}

test("Share Collection… is on a collection and inside it", async () => {
  await openArbesk();
  await rightClick(row("Shapes"));
  expect(menuLabels()).toContain("Share Collection…");
  await closeOverlays();

  await open("Shapes");
  await settle(20);
  await backgroundMenu();
  expect(menuLabels()).toContain("Share Collection…");
  await closeOverlays();
  await click($("#btn-location-menu"));
  await settle(5);
  expect(menuLabels()).toContain("Share Collection…");
  await closeOverlays();
  await openArbesk();
});

test("sharing adds a viewer, then changes the same account to editor", async () => {
  await openArbesk();
  await open("Shapes");
  await settle(20);
  await openShareDialog();
  expect(dialogHeading()).toBe("Share Collection");
  expect($(".dialog-layer .dialog-text").textContent).toBe("Add a collaborator to “Shapes” by the email they use with Arbesk.");
  const share = $$(".dialog-layer .dialog-responses button").find((button) => button.textContent === "Share")!;
  expect(share.hasAttribute("disabled")).toBe(true);

  await type($<HTMLInputElement>("#share-collection-email"), "alice@example.com");
  expect(share.hasAttribute("disabled")).toBe(false);
  await respond("Share");
  await vi.waitFor(() => expect(toasts()).toContain("Shared with alice@example.com as Viewer"), { timeout: 2000 });

  // Same role again: the dialog stays open with the reason.
  await openShareDialog();
  await type($<HTMLInputElement>("#share-collection-email"), "alice@example.com");
  await respond("Share");
  await settle(20);
  expect(dialogHeading()).toBe("Share Collection");
  expect($(".dialog-layer .field-message").textContent).toBe("That account is already a viewer.");
  await closeOverlays();

  // A different role changes the existing member.
  await openShareDialog();
  await type($<HTMLInputElement>("#share-collection-email"), "alice@example.com");
  const role = $<HTMLSelectElement>("#share-collection-role");
  role.value = "2";
  await respond("Share");
  await vi.waitFor(() => expect(toasts()).toContain("alice@example.com is now Editor"), { timeout: 2000 });
});

test("an unknown email stays in the dialog with the reason", async () => {
  await openArbesk();
  await open("Shapes");
  await settle(20);
  await openShareDialog();
  await type($<HTMLInputElement>("#share-collection-email"), "unknown@example.com");
  await respond("Share");
  await settle(20);
  expect(dialogHeading()).toBe("Share Collection");
  expect($(".dialog-layer .field-message").textContent).toBe(
    "No Arbesk account found for that email. They need to sign in to Arbesk at least once.",
  );
  await closeOverlays();
});

test("a session that expires during the share signs in and carries on", async () => {
  await openArbesk();
  await open("Shapes");
  await settle(20);
  arbeskMock().expireOnce();
  await openShareDialog();
  await type($<HTMLInputElement>("#share-collection-email"), "carol@example.com");
  await respond("Share");
  expect(dialogHeading()).toBe("Sign In to Arbesk");
  await respond("Open Browser");
  await vi.waitFor(() => expect(toasts()).toContain("Shared with carol@example.com as Viewer"), { timeout: 2000 });
});

test("sharing with no session signs in first; Cancel shares nothing", async () => {
  await openArbesk();
  await open("Shapes");
  await settle(20);
  arbeskMock().signedIn = false;
  await openShareDialog();
  await type($<HTMLInputElement>("#share-collection-email"), "bob@example.com");
  await respond("Share");
  expect(dialogHeading()).toBe("Sign In to Arbesk");
  await respondTop("Cancel");
  await settle(20);
  expect(dialogHeading()).toBeUndefined();
  expect(toasts()).not.toContain("Shared with bob@example.com as Viewer");

  await openShareDialog();
  await type($<HTMLInputElement>("#share-collection-email"), "bob@example.com");
  await respond("Share");
  expect(dialogHeading()).toBe("Sign In to Arbesk");
  await respond("Open Browser");
  await vi.waitFor(() => expect(toasts()).toContain("Shared with bob@example.com as Viewer"), { timeout: 2000 });
});

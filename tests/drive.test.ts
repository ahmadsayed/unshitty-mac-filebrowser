// The Google Drive location: connect through the mock, browse the fake Drive,
// and keep the read-only milestone free of version markers and write actions.
import { beforeAll, expect, test } from "vitest";
import { $, $$, boot, key, names, settle } from "./app";

beforeAll(async () => {
  await boot();
});

test("Google Drive connects and browses the mock drive", async () => {
  const driveRow = () =>
    $$("#sidebar-places .sidebar-row").find((row) => row.textContent?.includes("Google Drive"))!;
  expect(driveRow()).toBeTruthy();
  expect(driveRow().title).toBe("Connect to Google Drive");

  driveRow().click();
  await settle(10);
  expect($(".dialog-heading").textContent).toBe("Connect to Google Drive");

  const connect = [...document.querySelectorAll<HTMLButtonElement>(".dialog-responses button")].find(
    (button) => button.textContent === "Open Browser",
  )!;
  connect.click();
  await settle(30);

  expect($(".crumb.current")?.textContent).toContain("My Drive");
  expect(names()).toContain("Documents");
  expect(names()).toContain("hello.txt");

  const documents = $$("#view .item").find(
    (item) => item.querySelector(".name")?.textContent === "Documents",
  )!;
  documents.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  await settle(20);
  expect(names()).toEqual(["nested.txt"]);

  // Moving back up returns to My Drive, and the marker stays absent.
  await key("ArrowUp", { alt: true });
  await settle(20);
  expect(names()).toContain("hello.txt");
  // Opening a Drive file downloads a snapshot and shows the one-time notice.
  const hello = $$("#view .item").find(
    (item) => item.querySelector(".name")?.textContent === "hello.txt",
  )!;
  hello.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  await settle(10);
  expect(
    [...document.querySelectorAll(".toast-title")].some((toast) =>
      toast.textContent?.includes("Opened a local copy"),
    ),
  ).toBe(true);

  // The path bar accepts a Drive URI but rejects a bare relative name.
  await key("l", { mod: true });
  const pathEntry = document.querySelector<HTMLInputElement>("#path-entry")!;
  pathEntry.value = "Documents";
  pathEntry.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  await settle(10);
  expect(
    [...document.querySelectorAll(".toast-title")].some((toast) =>
      toast.textContent?.includes("Enter a Google Drive location"),
    ),
  ).toBe(true);
  pathEntry.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  await settle(5);

  await key("l", { mod: true });
  pathEntry.value = "gdrive://mock/mock-root~My%20Drive";
  pathEntry.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  await settle(20);
  expect(names()).toContain("hello.txt");

  expect($$(".versioned-icon").length).toBe(0);
  expect(driveRow().title).toBe("me@example.com");
});

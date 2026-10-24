// The Google Drive location: connect through the mock, browse the fake Drive,
// and keep the read-only milestone free of version markers and write actions.
import { beforeAll, expect, test } from "vitest";
import { $, $$, boot, click, closeOverlays, goTo, item, key, menuLabels, mouse, names, respond, selectedNames, settle, toasts, type } from "./app";

beforeAll(async () => {
  await boot();
});

test("Google Drive connects and browses the mock drive", async () => {
  const driveRow = () =>
    $$("#sidebar-places .sidebar-row").find((row) => row.textContent?.includes("Google Drive"))!;
  expect(driveRow()).toBeTruthy();
  expect(driveRow().title).toBe("Connect to Google Drive");

  // No OAuth client yet: the setup step comes first.
  driveRow().click();
  await settle(10);
  expect($(".dialog-heading").textContent).toBe("Add Your Google Client");
  const links = $$(".dialog-links button").map((button) => button.textContent);
  expect(links).toEqual(["Open Guide", "Choose File…"]);

  // A web client is refused with a hint, and the dialog stays open.
  const paste = $<HTMLTextAreaElement>(".client-json");
  paste.value = '{"web":{"client_id":"x.apps.googleusercontent.com"}}';
  await respond("Add Client");
  expect($(".dialog-error").textContent).toContain("Desktop app");
  expect($(".dialog-heading").textContent).toBe("Add Your Google Client");

  // Choosing the downloaded file adds the client and moves on.
  const input = $<HTMLInputElement>('.dialog input[type="file"]');
  const client = JSON.stringify({
    installed: { client_id: "123-abc.apps.googleusercontent.com", client_secret: "GOCSPX-test" },
  });
  Object.defineProperty(input, "files", {
    value: [new File([client], "client_secret.json", { type: "application/json" })],
    configurable: true,
  });
  input.dispatchEvent(new Event("change"));
  await settle(15);
  expect($(".dialog-heading").textContent).toBe("Connect to Google Drive");
  expect($$(".dialog-links button").map((button) => button.textContent)).toEqual([
    "Open Guide",
    "Change Client…",
  ]);

  await respond("Open Browser");
  await settle(15);

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

const DRIVE = "gdrive://mock/mock-root~My%20Drive";

async function select(name: string): Promise<void> {
  await click(item(name));
}

test("New Folder in Drive has no Keep versions option and creates the folder", async () => {
  await goTo(DRIVE);
  await key("N", { mod: true, shift: true });
  expect($(".dialog-layer").textContent).toContain("New Folder");
  expect(document.querySelector("#new-folder-versions")).toBeNull();
  await type($<HTMLInputElement>("#new-folder-name"), "Projects");
  await respond("Create");
  expect(names()).toContain("Projects");
});

test("Rename in Drive keeps the item selected under its new name", async () => {
  await goTo(DRIVE);
  await select("hello.txt");
  await key("F2");
  const input = $<HTMLInputElement>(".rename-popover input");
  await type(input, "greeting.txt");
  await key("Enter", { target: input });
  await settle(15);
  expect(names()).toContain("greeting.txt");
  expect(names()).not.toContain("hello.txt");
  expect(selectedNames()).toEqual(["greeting.txt"]);
});

test("Copy and Paste inside Drive asks about a clash and Keep Both adds (copy)", async () => {
  await goTo(DRIVE);
  await select("greeting.txt");
  await key("c", { mod: true });
  await goTo(`${DRIVE}/folder-1~Documents`);
  await key("v", { mod: true });
  await settle(15);
  expect(names()).toContain("greeting.txt");
  await key("v", { mod: true });
  await settle(15);
  await respond("Keep Both");
  expect(names()).toContain("greeting (copy).txt");
});

test("Move to Trash in Drive can be undone", async () => {
  await goTo(DRIVE);
  await select("Report.docx");
  await key("Backspace", { mod: true });
  await settle(15);
  expect(names()).not.toContain("Report.docx");
  expect(toasts().some((t) => t.includes("“Report.docx” moved to trash"))).toBe(true);
  const undo = $$(".toast button").find((b) => b.textContent === "Undo")!;
  await click(undo);
  await settle(15);
  expect(names()).toContain("Report.docx");
});

test("Paste from a local folder into Drive uploads the file", async () => {
  await goTo("/Users/me/Desktop");
  await select("todo.txt");
  await key("c", { mod: true });
  await goTo(DRIVE);
  await key("v", { mod: true });
  await settle(15);
  expect(names()).toContain("todo.txt");
});

test("Paste from Drive into a local folder downloads it", async () => {
  await goTo(DRIVE);
  await select("greeting.txt");
  await key("c", { mod: true });
  await goTo("/Users/me/Documents");
  await key("v", { mod: true });
  await settle(15);
  expect(names()).toContain("greeting.txt");
});

test("the Drive item menu offers writes and has no empty sections", async () => {
  await goTo(DRIVE);
  await select("Report.docx");
  await mouse(item("Report.docx"), "contextmenu");
  expect(menuLabels()).toEqual([
    "Open", "Open Item Location", "Cut", "Copy", "Rename…", "Move to Trash", "Properties",
  ]);
  const rows = $$(".popover .menu > *");
  const doubled = rows.some((row, i) => i > 0 && row.className === "menu-separator" && rows[i - 1].className === "menu-separator");
  expect(doubled).toBe(false);
  await closeOverlays();
});

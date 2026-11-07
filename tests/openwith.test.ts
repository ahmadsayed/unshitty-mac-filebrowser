// Open With: list installed applications, search them, and launch the chosen one.
import { afterEach, beforeAll, expect, test } from "vitest";
import { $, $$, boot, chooseMenu, click, closeOverlays, dialogHeading, goTo, item, key, mouse, names, settle, type } from "./app";

const HOME = "/Users/me";
const NOTES = `${HOME}/Documents/Notes.md`;

type MockApps = {
  set(o: { list?: { name: string; id: string }[]; failList?: string; failOpen?: string }): void;
  hold(): void; release(): void; opened(): { path: string; app: string }[]; reset(): void;
};
const mockApps = () => (globalThis as unknown as { __mockApps: MockApps }).__mockApps;

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
});

afterEach(async () => {
  await closeOverlays();
  mockApps().reset();
});

async function rightClick(el: Element): Promise<void> {
  await mouse(el, "mousedown", { button: 2 });
  await mouse(el, "contextmenu", { button: 2 });
}

async function openWithDialog(): Promise<void> {
  await goTo(`${HOME}/Documents`);
  await rightClick(item("Notes.md"));
  await chooseMenu("Open With…");
  await settle(20);
}

const search = () => $<HTMLInputElement>(".app-search");
const choiceNames = () => $$(".app-choice:not([hidden]) .app-choice-name").map((el) => el.textContent);
const selectedName = () => $(".app-choice[aria-selected='true'] .app-choice-name")?.textContent;
const status = () => $(".app-status:not([hidden])")?.textContent;
const openButton = () => $$(".dialog-layer .dialog-responses button").find((b) => b.textContent === "Open") as HTMLButtonElement;

test("Open With… lists applications and closes after choosing one", async () => {
  await openWithDialog();
  expect(dialogHeading()).toBe("Open “Notes.md” With");
  expect($$(".app-choice").map((button) => button.textContent)).toEqual(
    expect.arrayContaining(["TextEdit", "Preview"])
  );

  const textEdit = $$(".app-choice").find((button) => button.textContent === "TextEdit")!;
  await click(textEdit);
  await settle(20);
  expect(document.querySelector(".dialog-layer")).toBeNull();
  expect(mockApps().opened()).toEqual([{ path: NOTES, app: "/Applications/TextEdit.app" }]);
});

test("typing narrows the list, ignoring case and accents, and clearing restores it", async () => {
  mockApps().set({ list: [
    { name: "TextEdit", id: "/Applications/TextEdit.app" },
    { name: "Éditeur de texte", id: "/Applications/Editeur.app" },
    { name: "Preview", id: "/Applications/Preview.app" },
  ] });
  await openWithDialog();
  expect(document.activeElement).toBe(search());

  await type(search(), "EDIT");
  expect(choiceNames()).toEqual(["TextEdit", "Éditeur de texte"]);
  await type(search(), "editeur");
  expect(choiceNames()).toEqual(["Éditeur de texte"]);
  await type(search(), "");
  expect(choiceNames()).toEqual(["TextEdit", "Éditeur de texte", "Preview"]);
  expect(status()).toBeUndefined();
});

test("a search with no match says so and offers nothing to open", async () => {
  await openWithDialog();
  await type(search(), "zzz");
  expect(choiceNames()).toEqual([]);
  expect(status()).toBe("No applications match “zzz”.");
  expect(openButton().disabled).toBe(true);
  await key("Enter", { target: search() });
  expect(document.querySelector(".dialog-layer")).not.toBeNull();
  expect(mockApps().opened()).toEqual([]);
});

test("loading, no applications and failed discovery each have their own message", async () => {
  mockApps().hold();
  await openWithDialog();
  expect(status()).toBe("Loading applications…");
  // Typing can start before the list arrives.
  expect(search().disabled).toBe(false);
  await type(search(), "view");
  expect(openButton().disabled).toBe(true);
  mockApps().release();
  await settle(20);
  expect(status()).toBeUndefined();
  expect(choiceNames()).toEqual(["Preview"]);
  await closeOverlays();

  mockApps().set({ list: [] });
  await openWithDialog();
  expect(status()).toBe("No applications found.");
  expect(search().disabled).toBe(true);
  await closeOverlays();

  mockApps().set({ failList: "Permission denied" });
  await openWithDialog();
  expect(status()).toBe("Could not list applications: Permission denied");
  expect(search().disabled).toBe(true);
  expect(openButton().disabled).toBe(true);
});

test("arrow keys move the selection and Enter opens the selected application", async () => {
  await openWithDialog();
  expect(selectedName()).toBe("TextEdit");
  await key("ArrowDown", { target: search() });
  expect(selectedName()).toBe("Preview");
  await key("ArrowDown", { target: search() });
  await key("ArrowDown", { target: search() });
  expect(selectedName()).toBe("Visual Studio Code");
  await key("ArrowUp", { target: search() });
  expect(selectedName()).toBe("Preview");
  expect(document.activeElement).toBe(search());

  await key("Enter", { target: search() });
  await settle(20);
  expect(document.querySelector(".dialog-layer")).toBeNull();
  expect(mockApps().opened()).toEqual([{ path: NOTES, app: "/Applications/Preview.app" }]);
});

test("Escape closes without launching anything", async () => {
  await openWithDialog();
  await type(search(), "pre");
  await key("Escape", { target: search() });
  expect(document.querySelector(".dialog-layer")).toBeNull();
  expect(mockApps().opened()).toEqual([]);
});

test("the selection stays while its application is shown, and moves to the first one when hidden", async () => {
  await openWithDialog();
  await key("ArrowDown", { target: search() });
  expect(selectedName()).toBe("Preview");
  await type(search(), "e");
  expect(selectedName()).toBe("Preview");
  await type(search(), "text");
  expect(choiceNames()).toEqual(["TextEdit"]);
  expect(selectedName()).toBe("TextEdit");
  await type(search(), "");
  expect(selectedName()).toBe("TextEdit");
});

test("each application appears once by id, and apps sharing a name stay apart", async () => {
  mockApps().set({ list: [
    { name: "Editor", id: "/Applications/Editor.app" },
    { name: "Editor", id: "/Applications/Editor.app" },
    { name: "Editor", id: `${HOME}/Applications/Editor.app` },
  ] });
  await openWithDialog();
  expect(choiceNames()).toEqual(["Editor", "Editor"]);
  expect($$(".app-choice .app-choice-detail").map((el) => el.textContent)).toEqual([
    "/Applications/Editor.app",
    `${HOME}/Applications/Editor.app`,
  ]);
  await key("ArrowDown", { target: search() });
  await key("Enter", { target: search() });
  await settle(20);
  expect(mockApps().opened()).toEqual([{ path: NOTES, app: `${HOME}/Applications/Editor.app` }]);
});

test("a failed launch keeps the chooser open and can be retried", async () => {
  mockApps().set({ failOpen: "The application can't be opened." });
  await openWithDialog();
  await key("Enter", { target: search() });
  await settle(20);
  expect(dialogHeading()).toBe("Open “Notes.md” With");
  expect($(".open-with-dialog .field-message").textContent).toBe("The application can't be opened.");
  expect(choiceNames()).toEqual(["TextEdit", "Preview", "Visual Studio Code"]);

  await click(openButton());
  await settle(20);
  expect(document.querySelector(".dialog-layer")).toBeNull();
  expect(mockApps().opened()).toEqual([{ path: NOTES, app: "/Applications/TextEdit.app" }]);
});

type MockCalls = { count(cmd: string): number; hold(cmd: string): void; release(cmd: string): void; reset(): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;
const cancelButton = () => $$(".dialog-layer .dialog-responses button").find((b) => b.textContent === "Cancel") as HTMLButtonElement;

test("a launch in progress ignores a second Enter, Open or row, and Escape still closes", async () => {
  calls().reset();
  await openWithDialog();
  calls().hold("open_with");
  await key("Enter", { target: search() });
  expect(openButton().disabled).toBe(true);
  expect(cancelButton().disabled).toBe(false);
  await key("Enter", { target: search() });
  await click(openButton());
  await click($$(".app-choice").find((button) => button.textContent === "Preview")!);
  expect(calls().count("open_with")).toBe(1);
  await key("Escape", { target: search() });
  expect(document.querySelector(".dialog-layer")).toBeNull();
  calls().release("open_with");
  await settle(20);
  expect(mockApps().opened()).toEqual([{ path: NOTES, app: "/Applications/TextEdit.app" }]);
});

test("a failed launch enables Open and every application again", async () => {
  mockApps().set({ failOpen: "The application can't be opened." });
  await openWithDialog();
  await key("Enter", { target: search() });
  await settle(20);
  expect($(".open-with-dialog .field-message").textContent).toBe("The application can't be opened.");
  expect(openButton().disabled).toBe(false);
  expect($$(".app-choice").map((button) => (button as HTMLButtonElement).disabled)).toEqual([false, false, false]);
  expect(search().disabled).toBe(false);
});

// ----- Folders -----

const chooseApp = async (name: string) => {
  await click($$(".app-choice").find((button) => button.textContent === name)!);
  await settle(20);
};
const menuItem = (label: string) => $$(".popover .menu-item").find((b) => b.textContent === label) as HTMLButtonElement | undefined;

test("a folder can be opened with any application; Preview stays for files", async () => {
  await goTo(HOME);
  await rightClick(item("Empty folder"));
  expect(menuItem("Open With…")?.disabled).toBe(false);
  expect(menuItem("Preview")?.disabled).toBe(true);
  await chooseMenu("Open With…");
  await settle(20);
  expect(dialogHeading()).toBe("Open “Empty folder” With");
  await chooseApp("TextEdit");
  expect(mockApps().opened()).toEqual([{ path: `${HOME}/Empty folder`, app: "/Applications/TextEdit.app" }]);
  // Like a file, it joins Recent: a list of recent projects for free.
  await click($$(".sidebar-row").find((r) => r.textContent?.trim() === "Recent")!);
  await settle(10);
  expect(names()[0]).toBe("Empty folder");
});

test("the current folder opens with an application from the background menu", async () => {
  await goTo(`${HOME}/Documents`);
  await rightClick($("#view"));
  await chooseMenu("Open With…");
  await settle(20);
  expect(dialogHeading()).toBe("Open “Documents” With");
  await chooseApp("Preview");
  expect(mockApps().opened()).toEqual([{ path: `${HOME}/Documents`, app: "/Applications/Preview.app" }]);
});

test("the current folder opens with an application from the path-bar menu", async () => {
  await goTo(`${HOME}/Documents`);
  await click($("#btn-location-menu"));
  await settle();
  await chooseMenu("Open With…");
  await settle(20);
  expect(dialogHeading()).toBe("Open “Documents” With");
  await chooseApp("TextEdit");
  expect(mockApps().opened()).toEqual([{ path: `${HOME}/Documents`, app: "/Applications/TextEdit.app" }]);
});

test("places without a folder of their own don't offer Open With… for it", async () => {
  for (const place of ["Recent", "Starred", "Network"]) {
    await click($$(".sidebar-row").find((r) => r.textContent?.trim() === place)!);
    await settle(10);
    await rightClick($("#view"));
    expect(menuItem("Open With…")).toBeUndefined();
    await closeOverlays();
    await click($("#btn-location-menu"));
    await settle();
    expect(menuItem("Open With…")).toBeUndefined();
    await closeOverlays();
  }
  await goTo(`${HOME}/.Trash`);
  await rightClick($("#view"));
  expect(menuItem("Open With…")).toBeUndefined();
  await closeOverlays();
});

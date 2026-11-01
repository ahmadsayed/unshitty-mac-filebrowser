// "Open With…": the installed applications, searchable by name, opened by a
// click, or by the arrow keys and Enter (GNOME Files 51's searchable chooser).
import { listApps, openWith, type AppChoice, type Entry } from "./api";
import { openDialog, type DialogHandle } from "./ui";

/** What the dialog needs from the rest of the app. */
export interface OpenWithHooks {
  /** Fill a row's 28 px icon for `app`. */
  icon(span: HTMLElement, app: AppChoice): void;
  /** The message to show for a failed command. */
  errorText(err: unknown): string;
  /** The file was handed to an application. */
  opened(entry: Entry): void;
}

/** Text as search compares it: lower case, without accents. */
export function foldText(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

/** The applications in order, each id once. */
export function uniqueApps(apps: AppChoice[]): AppChoice[] {
  const seen = new Set<string>();
  return apps.filter((app) => !seen.has(app.id) && seen.add(app.id));
}

/** Names more than one application has: their rows also show the id. */
export function sharedNames(apps: AppChoice[]): Set<string> {
  const counts = new Map<string, number>();
  for (const app of apps) counts.set(app.name, (counts.get(app.name) ?? 0) + 1);
  return new Set([...counts].filter(([, n]) => n > 1).map(([name]) => name));
}

interface Row {
  app: AppChoice;
  button: HTMLButtonElement;
  /** The folded name search matches against. */
  key: string;
}

function rowButton(app: AppChoice, detail: boolean, hooks: OpenWithHooks): HTMLButtonElement {
  const button = document.createElement("button");
  button.className = "app-choice";
  button.setAttribute("role", "option");
  button.setAttribute("aria-selected", "false");
  button.tabIndex = -1;
  const icon = document.createElement("span");
  icon.className = "app-choice-icon";
  hooks.icon(icon, app);
  const text = document.createElement("span");
  text.className = "app-choice-text";
  const name = document.createElement("span");
  name.className = "app-choice-name";
  name.textContent = app.name;
  text.append(name);
  if (detail) {
    const id = document.createElement("span");
    id.className = "app-choice-detail";
    id.textContent = app.id;
    text.append(id);
  }
  button.append(icon, text);
  return button;
}

/** The dialog's parts and state. */
class Chooser {
  readonly body = document.createElement("div");
  readonly search = document.createElement("input");
  readonly status = document.createElement("p");
  readonly list = document.createElement("div");
  readonly message = document.createElement("div");
  dialog!: DialogHandle;
  rows: Row[] = [];
  selected: Row | null = null;
  launching = false;

  constructor(readonly entry: Entry, readonly hooks: OpenWithHooks) {
    this.body.className = "open-with-body";
    this.search.className = "app-search";
    this.search.type = "search";
    this.search.placeholder = "Search Applications";
    this.search.setAttribute("aria-label", "Search Applications");
    this.search.spellcheck = false;
    this.search.autocomplete = "off";
    this.status.className = "app-status";
    this.list.className = "app-list";
    this.list.setAttribute("role", "listbox");
    this.message.className = "field-message";
    this.body.append(this.search, this.status, this.list, this.message);
    this.search.addEventListener("input", () => this.filter());
    this.body.addEventListener("keydown", (e) => this.onKey(e));
    this.setStatus("Loading applications…");
  }

  setStatus(text: string | null): void {
    this.status.hidden = text === null;
    this.status.textContent = text ?? "";
  }

  /** Nothing to choose from: say why, and leave nothing to type into. */
  fail(text: string): void {
    this.setStatus(text);
    this.search.disabled = true;
  }

  fill(apps: AppChoice[]): void {
    const unique = uniqueApps(apps);
    if (unique.length === 0) return this.fail("No applications found.");
    const shared = sharedNames(unique);
    this.rows = unique.map((app) => ({ app, button: rowButton(app, shared.has(app.name), this.hooks), key: foldText(app.name) }));
    for (const row of this.rows) {
      row.button.addEventListener("click", () => {
        this.select(row);
        this.launch();
      });
      row.button.addEventListener("focus", () => this.select(row));
      this.list.append(row.button);
    }
    this.filter();
  }

  visible(): Row[] {
    return this.rows.filter((row) => !row.button.hidden);
  }

  /** Show the rows matching the search; the selection stays if it's still shown. */
  filter(): void {
    if (this.rows.length === 0) return;
    const query = this.search.value.trim();
    const folded = foldText(query);
    for (const row of this.rows) row.button.hidden = !row.key.includes(folded);
    const shown = this.visible();
    if (!this.selected || this.selected.button.hidden) this.select(shown[0] ?? null);
    this.setStatus(shown.length ? null : `No applications match “${query}”.`);
  }

  select(row: Row | null): void {
    if (this.selected) this.selected.button.setAttribute("aria-selected", "false");
    this.selected = row;
    if (row) {
      row.button.setAttribute("aria-selected", "true");
      this.reveal(row.button);
    }
    this.updateOpen();
  }

  /** Scroll the list just enough to show `button`. */
  reveal(button: HTMLElement): void {
    const top = button.offsetTop;
    const bottom = top + button.offsetHeight;
    if (top < this.list.scrollTop) this.list.scrollTop = top;
    else if (bottom > this.list.scrollTop + this.list.clientHeight) this.list.scrollTop = bottom - this.list.clientHeight;
  }

  move(step: number): void {
    const shown = this.visible();
    if (shown.length === 0) return;
    const at = this.selected ? shown.indexOf(this.selected) : -1;
    this.select(shown[Math.max(0, Math.min(shown.length - 1, at + step))]);
  }

  onKey(e: KeyboardEvent): void {
    const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    this.move(step);
  }

  updateOpen(): void {
    this.dialog?.setResponseEnabled("open", !!this.selected && !this.launching);
  }

  /** Open the file with the selected application; a failure keeps the dialog for a retry. */
  launch(): void {
    const row = this.selected;
    if (!row || this.launching) return;
    this.launching = true;
    row.button.disabled = true;
    this.message.textContent = "";
    this.updateOpen();
    openWith(this.entry.path, row.app.id)
      .then(() => {
        this.dialog.close();
        this.hooks.opened(this.entry);
      })
      .catch((err) => {
        this.message.textContent = this.hooks.errorText(err);
      })
      .finally(() => {
        this.launching = false;
        row.button.disabled = false;
        this.updateOpen();
      });
  }
}

/** A searchable list of installed applications for "Open With…". */
export function showOpenWith(entry: Entry, hooks: OpenWithHooks): void {
  const chooser = new Chooser(entry, hooks);
  chooser.dialog = openDialog({
    heading: `Open “${entry.name}” With`,
    extra: chooser.body,
    className: "open-with-dialog",
    responses: [
      { id: "cancel", label: "Cancel" },
      { id: "open", label: "Open", style: "suggested" },
    ],
    defaultResponse: "open",
    closeResponse: "cancel",
    onResponse: (id) => {
      if (id !== "open") return;
      chooser.launch();
      return false;
    },
  });
  chooser.updateOpen();
  chooser.search.focus();

  void listApps()
    .then((apps) => {
      if (chooser.body.isConnected) chooser.fill(apps);
    })
    .catch((err) => {
      if (chooser.body.isConnected) chooser.fail(`Could not list applications: ${hooks.errorText(err)}`);
    });
}

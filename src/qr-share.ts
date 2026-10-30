// Share one file with a QR code: a phone scans it and downloads the file
// over iroh, on any network (src-tauri/src/share/file_share.rs). The share
// lasts while the dialog is open; closing it, or the window, stops it.
import { fileSharePrepare, fileShareStatus, fileShareStop, type Entry, type FileShareView } from "./api";
import { openDialog, showToast, type MenuItem } from "./ui";

/** How often the dialog asks for progress and downloads. */
const POLL_MS = 500;

/** Shares this window started and hasn't stopped yet. */
const running = new Set<string>();
window.addEventListener("pagehide", () => {
  for (const id of running) void fileShareStop(id).catch(() => {});
});

/** "Share with QR Code…" for one local file (not in the Trash). */
export function qrShareItems(entry: Entry | null, enabled: boolean): MenuItem[] {
  if (!enabled || !entry || entry.is_dir || !entry.path.startsWith("/")) return [];
  return [{ label: "Share with QR Code…", action: () => void showQrShare(entry) }];
}

export function downloadsText(count: number): string {
  if (count === 0) return "Waiting for a phone…";
  return count === 1 ? "Downloaded by 1 device" : `Downloaded by ${count} devices`;
}

/** What the status line says for a view. */
export function statusText(view: FileShareView): string {
  switch (view.state) {
    case "preparing":
      return view.progress < 1 ? `Preparing… ${Math.floor(view.progress * 100)}%` : "Connecting…";
    case "failed":
      return view.error;
    case "ready":
      return view.sending !== null ? `Sending… ${Math.floor(view.sending * 100)}%` : downloadsText(view.downloads);
  }
}

export async function showQrShare(entry: Entry): Promise<void> {
  let id: string;
  try {
    id = await fileSharePrepare(entry.path);
  } catch (err) {
    showToast(`Could not share “${entry.name}”: ${String(err)}`);
    return;
  }
  running.add(id);

  const extra = document.createElement("div");
  extra.className = "qr-share";
  extra.innerHTML =
    `<div class="qr-share-code"><div class="qr-share-placeholder"><div class="operation-bar"><div class="operation-bar-fill"></div></div></div></div>` +
    `<p class="qr-share-hint">Scan with a phone’s camera to download the file. Works on any network while this window is open.</p>` +
    `<div class="qr-share-link" hidden><input class="entry" type="text" readonly spellcheck="false" aria-label="Link" /><button class="btn" type="button">Copy</button></div>` +
    `<p class="qr-share-status" role="status"></p>`;
  const code = extra.querySelector<HTMLElement>(".qr-share-code")!;
  const fill = extra.querySelector<HTMLElement>(".operation-bar-fill")!;
  const linkRow = extra.querySelector<HTMLElement>(".qr-share-link")!;
  const input = linkRow.querySelector("input")!;
  const status = extra.querySelector<HTMLElement>(".qr-share-status")!;
  const hint = extra.querySelector<HTMLElement>(".qr-share-hint")!;
  linkRow.querySelector("button")!.addEventListener("click", () => {
    void navigator.clipboard.writeText(input.value).then(() => showToast("Link copied"));
  });

  let shown = "";
  let timer = 0;
  const stop = () => {
    window.clearInterval(timer);
    if (running.delete(id)) void fileShareStop(id).catch(() => {});
  };
  const dialog = openDialog({
    heading: `Share “${entry.name}”`,
    extra,
    className: "qr-share-dialog",
    responses: [{ id: "stop", label: "Stop Sharing" }],
    defaultResponse: "stop",
    closeResponse: "stop",
    onResponse: () => stop(),
  });

  const update = async () => {
    let view: FileShareView | null;
    try {
      view = await fileShareStatus(id);
    } catch {
      view = null;
    }
    if (!running.has(id)) return;
    if (!view) {
      stop();
      dialog.close();
      return;
    }
    status.textContent = statusText(view);
    status.classList.toggle("error", view.state === "failed");
    if (view.state === "preparing") fill.style.width = `${Math.round(view.progress * 100)}%`;
    if (view.state === "failed") {
      window.clearInterval(timer);
      // Nothing to scan: only the reason stays.
      code.hidden = true;
      hint.hidden = true;
    }
    if (view.state === "ready" && shown !== view.link) {
      // The SVG is the backend's own drawing of the link: shown as an image,
      // never inserted as markup.
      shown = view.link;
      const img = document.createElement("img");
      img.className = "qr-share-image";
      img.alt = "QR code";
      img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(view.qr)}`;
      code.replaceChildren(img);
      input.value = view.link;
      linkRow.hidden = false;
    }
  };
  timer = window.setInterval(() => void update(), POLL_MS);
  await update();
}

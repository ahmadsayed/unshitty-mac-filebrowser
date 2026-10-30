// The download page: the ticket after `#` names the file (see
// src-tauri/src/share/file_link.rs). Download on a tap, keep the pieces in
// memory, then let the phone save or share the file: saving from a tap
// works everywhere, including iPhones.
import init, { download, ticket_size } from "./file_get.js";
import { isImage, showImage, typeOf } from "./preview.js";

const $ = (id) => document.getElementById(id);
const ticket = location.hash.slice(1);
let file = null;
let url = null;

/** GNOME-style sizes: "4 bytes", "12.3 kB", "4.5 MB". */
function formatSize(bytes) {
  if (bytes < 1000) return `${bytes} ${bytes === 1 ? "byte" : "bytes"}`;
  const units = ["kB", "MB", "GB", "TB"];
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

function show({ status, error = false, progress = null, buttons = [] }) {
  $("status").textContent = status;
  $("status").classList.toggle("error", error);
  $("bar").hidden = progress === null;
  if (progress !== null) $("fill").style.width = `${Math.round(progress * 100)}%`;
  for (const id of ["start", "save", "share", "retry"]) $(id).hidden = !buttons.includes(id);
}

/** The download in progress: pieces so far, bytes received, the size. */
const progress = { chunks: [], received: 0, total: 0 };

const messageOf = (err) => err?.message ?? String(err);

function started(name, size) {
  progress.total = size;
  $("name").textContent = name;
  document.title = name;
  show({ status: `Downloading… 0 bytes of ${formatSize(size)}`, progress: 0 });
}

function arrived(chunk) {
  progress.chunks.push(chunk);
  progress.received += chunk.length;
  const { received, total } = progress;
  show({ status: `Downloading… ${formatSize(received)} of ${formatSize(total)}`, progress: total ? received / total : 1 });
}

function finished(name) {
  file = new File(progress.chunks, name, { type: typeOf(name) });
  progress.chunks = [];
  if (isImage(file.type)) {
    url = URL.createObjectURL(file);
    showImage($("icon"), url, name);
  }
  const buttons = navigator.canShare?.({ files: [file] }) ? ["save", "share"] : ["save"];
  show({ status: "Downloaded and checked.", progress: 1, buttons });
}

function stillWaiting() {
  if (!progress.total) show({ status: "Still trying to reach the computer… It has to be on, with its QR code showing.", progress: 0 });
}

async function start() {
  Object.assign(progress, { chunks: [], received: 0, total: 0 });
  show({ status: "Connecting to the computer…", progress: 0 });
  const slow = setTimeout(stillWaiting, 15000);
  try {
    finished(await download(ticket, started, arrived));
  } catch (err) {
    show({ status: messageOf(err), error: true, buttons: ["retry"] });
  } finally {
    clearTimeout(slow);
  }
}

function save() {
  if (!file) return;
  url ??= URL.createObjectURL(file);
  const link = document.createElement("a");
  link.href = url;
  link.download = file.name;
  document.body.appendChild(link);
  link.click();
  link.remove();
}

async function share() {
  try {
    await navigator.share({ files: [file], title: file.name });
  } catch {
    /* cancelled */
  }
}

$("start").addEventListener("click", start);
$("retry").addEventListener("click", start);
$("save").addEventListener("click", save);
$("share").addEventListener("click", share);

if (!ticket) {
  show({ status: "Scan the QR code on the computer sharing a file to download it here." });
} else {
  try {
    if (typeof WebAssembly !== "object") throw new Error("This browser can't download shared files. Try another browser.");
    await init();
    const size = ticket_size(ticket);
    // The file is kept in memory until it is saved.
    $("size").textContent = size > 1e9 ? `${formatSize(size)} · the phone needs that much free memory` : formatSize(size);
    show({ status: "", buttons: ["start"] });
  } catch (err) {
    show({ status: messageOf(err), error: true });
  }
}

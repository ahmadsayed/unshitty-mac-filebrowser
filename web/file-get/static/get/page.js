// The download page: the ticket after `#` names the shared files (see
// src-tauri/src/share/file_link.rs). Download on a tap, keep the pieces in
// memory, then let the phone save or share the files: saving from a tap
// works everywhere, including iPhones.
import init, { download, ticket_files, ticket_size } from "./file_get.js";
import { fileRow, formatSize, isImage, showImage, typeOf } from "./preview.js";

const $ = (id) => document.getElementById(id);
const ticket = location.hash.slice(1);
/** The downloaded files, and an object URL per file once one is needed. */
let files = [];
const urls = new Map();

function show({ status, error = false, progress = null, buttons = [] }) {
  $("status").textContent = status;
  $("status").classList.toggle("error", error);
  $("bar").hidden = progress === null;
  if (progress !== null) $("fill").style.width = `${Math.round(progress * 100)}%`;
  for (const id of ["start", "save", "share", "retry"]) $(id).hidden = !buttons.includes(id);
}

/** The download in progress: the names, pieces per file, bytes received, the total. */
const progress = { names: [], chunks: [], received: 0, total: 0 };

const messageOf = (err) => err?.message ?? String(err);

function title(text) {
  $("name").textContent = text;
  document.title = text;
}

function listed(names, sizes) {
  progress.names = names;
  progress.chunks = names.map(() => []);
  progress.total = sizes.reduce((sum, size) => sum + size, 0);
  if (names.length === 1) title(names[0]);
  show({ status: `Downloading… 0 bytes of ${formatSize(progress.total)}`, progress: 0 });
}

function arrived(index, chunk) {
  progress.chunks[index].push(chunk);
  progress.received += chunk.length;
  const { received, total } = progress;
  show({ status: `Downloading… ${formatSize(received)} of ${formatSize(total)}`, progress: total ? received / total : 1 });
}

const urlOf = (file) => {
  if (!urls.has(file)) urls.set(file, URL.createObjectURL(file));
  return urls.get(file);
};

function finished() {
  files = progress.names.map((name, i) => new File(progress.chunks[i], name, { type: typeOf(name) }));
  progress.chunks = [];
  if (files.length === 1) {
    if (isImage(files[0].type)) showImage($("icon"), urlOf(files[0]), files[0].name);
  } else {
    $("files").replaceChildren(...files.map((file) => fileRow(file, urlOf(file), saveFile).row));
    $("files").hidden = false;
    $("save").textContent = "Save All";
  }
  const buttons = navigator.canShare?.({ files }) ? ["save", "share"] : ["save"];
  show({ status: "Downloaded and checked.", progress: 1, buttons });
}

function stillWaiting() {
  if (!progress.total) show({ status: "Still trying to reach the computer… It has to be on, with its QR code showing.", progress: 0 });
}

async function start() {
  Object.assign(progress, { names: [], chunks: [], received: 0, total: 0 });
  show({ status: "Connecting to the computer…", progress: 0 });
  const slow = setTimeout(stillWaiting, 15000);
  try {
    await download(ticket, listed, arrived);
    finished();
  } catch (err) {
    show({ status: messageOf(err), error: true, buttons: ["retry"] });
  } finally {
    clearTimeout(slow);
  }
}

function saveFile(file) {
  const link = document.createElement("a");
  link.href = urlOf(file);
  link.download = file.name;
  document.body.appendChild(link);
  link.click();
  link.remove();
}

function saveAll() {
  files.forEach(saveFile);
}

async function share() {
  try {
    await navigator.share({ files, title: files.length === 1 ? files[0].name : $("name").textContent });
  } catch {
    /* cancelled */
  }
}

$("start").addEventListener("click", start);
$("retry").addEventListener("click", start);
$("save").addEventListener("click", saveAll);
$("share").addEventListener("click", share);

if (!ticket) {
  show({ status: "Scan the QR code on the computer sharing a file to download it here." });
} else {
  try {
    if (typeof WebAssembly !== "object") throw new Error("This browser can't download shared files. Try another browser.");
    await init();
    const size = ticket_size(ticket);
    const count = ticket_files(ticket);
    if (count > 1) title(`${count} Files`);
    // The files are kept in memory until they are saved.
    $("size").textContent = size > 1e9 ? `${formatSize(size)} · the phone needs that much free memory` : formatSize(size);
    show({ status: "", buttons: ["start"] });
  } catch (err) {
    show({ status: messageOf(err), error: true });
  }
}

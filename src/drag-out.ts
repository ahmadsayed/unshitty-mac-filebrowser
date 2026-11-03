// ---------------------------------------------------------------------------
// Dragging files out of the window, to Finder, GNOME Files or a browser
//
// Inside the window a drag runs on mouse events (see main.ts). Once the
// pointer passes the window's edge with the button still held, the system
// drag takes over: on Linux through our own GTK command (start_file_drag),
// on macOS through the drag plugin. Both offer a copy only.
// ---------------------------------------------------------------------------
import { startFileDrag } from "./api";

/** A 1×1 transparent PNG, the drag image the macOS plugin needs. It must
 * decode: the plugin gives up on an image NSImage refuses (tested). */
export const DRAG_IMAGE =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR4nGNgYAAAAAMAAbitOmMAAAAASUVORK5CYII=";

/** Whether a move during a drag is past the window's edge. WebKit keeps
 * sending moves there while the button is held, but sends no mouseleave
 * until the button is released, which is too late to start a system drag. */
export function leftWindow(e: MouseEvent): boolean {
  return e.clientX < 0 || e.clientY < 0 || e.clientX >= window.innerWidth || e.clientY >= window.innerHeight;
}

/** The items of the drag out of this window in progress. If the system drag
 * comes back and is dropped here, it is still the window's own drag, not one
 * from another app. Forgotten when the drag ends (GTK's drag-end on Linux,
 * the plugin's callback on macOS), on the next click in the page, or once a
 * drop back here is handled. */
let lastDragOut: string[] | null = null;

/** Remember the items handed to the system drag. */
export function noteDragOut(paths: string[]): void {
  lastDragOut = paths;
}

/** Whether a drop carries exactly the items of the last drag out. */
export function isOwnDragOut(paths: string[]): boolean {
  return !!lastDragOut && lastDragOut.length === paths.length && lastDragOut.every((p, i) => p === paths[i]);
}

export function forgetDragOut(): void {
  lastDragOut = null;
}

// During a system drag the page gets no clicks, so a click means it ended.
window.addEventListener("mousedown", forgetDragOut, true);

/** The system drag ended. A drop on this window reaches the page just after
 * the end notice: wry queues the drop event, while the notice runs at once.
 * So when the drop may have landed here, keep the drag for that drop, which
 * forgets it itself, and at most a few seconds. */
function dragEnded(mayHaveLandedHere: boolean): void {
  if (!mayHaveLandedHere) return forgetDragOut();
  const paths = lastDragOut;
  window.setTimeout(() => lastDragOut === paths && forgetDragOut(), 3000);
}

/** What the drag plugin reports when a macOS drag session ends. It doesn't
 * say where a drop landed. */
interface PluginDragEnd {
  result: "Dropped" | "Cancel";
}

interface DragPlugin {
  startDrag?: (
    options: { item: string[]; icon: string; mode: "copy" | "move" },
    onEvent?: (end: PluginDragEnd) => void,
  ) => Promise<void>;
}

/** Hand local `paths` to the system drag, remembering them until it ends.
 * Rejects with a message when it can't start. */
export async function startSystemDrag(paths: string[]): Promise<void> {
  noteDragOut(paths);
  try {
    if (/Linux/.test(navigator.userAgent)) {
      // No drag when the button was released before it could start.
      // GTK tells whether the drop landed on this window.
      if (!(await startFileDrag(paths, dragEnded))) forgetDragOut();
      return;
    }
    const plugin = (window as unknown as { __TAURI__?: { drag?: DragPlugin } }).__TAURI__?.drag;
    if (!plugin?.startDrag) throw "Dragging out isn't available.";
    await plugin.startDrag({ item: paths, icon: DRAG_IMAGE, mode: "copy" }, (end) => dragEnded(end?.result === "Dropped"));
  } catch (err) {
    forgetDragOut();
    throw err;
  }
}

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

interface DragPlugin {
  startDrag?: (options: { item: string[]; icon: string; mode: "copy" | "move" }) => Promise<void>;
}

/** Hand local `paths` to the system drag. Rejects with a message when it can't start. */
export async function startSystemDrag(paths: string[]): Promise<void> {
  if (/Linux/.test(navigator.userAgent)) return startFileDrag(paths);
  const plugin = (window as unknown as { __TAURI__?: { drag?: DragPlugin } }).__TAURI__?.drag;
  if (!plugin?.startDrag) throw "Dragging out isn't available.";
  await plugin.startDrag({ item: paths, icon: DRAG_IMAGE, mode: "copy" });
}

// Browser-only stand-in for @tauri-apps/api/webview (see core.ts).
// `window.__mockDrop(paths, x, y)` simulates files dropped from Finder.
type Handler = (event: { payload: unknown }) => void;
let handler: Handler | null = null;

export const getCurrentWebview = () => ({
  onDragDropEvent: async (h: Handler) => {
    handler = h;
    return () => (handler = null);
  },
});

(window as unknown as Record<string, unknown>).__mockDrop = (paths: string[], x: number, y: number) => {
  // wry reports the position in CSS pixels on macOS and Linux (tauri only
  // labels it physical), so pass it on unscaled, as the real webview does.
  const position = { x, y };
  handler?.({ payload: { type: "enter", paths, position } });
  handler?.({ payload: { type: "over", position } });
  handler?.({ payload: { type: "drop", paths, position } });
};

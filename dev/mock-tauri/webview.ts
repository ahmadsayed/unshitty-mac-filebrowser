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
  const position = { x: x * devicePixelRatio, y: y * devicePixelRatio };
  handler?.({ payload: { type: "enter", paths, position } });
  handler?.({ payload: { type: "over", position } });
  handler?.({ payload: { type: "drop", paths, position } });
};

// Turning and zooming the 3D preview with a pointer, without three.js: a
// drag turns the model; the wheel and a pinch zoom it. A pinch arrives three
// ways: two touch points, a trackpad's Ctrl/⌘ + wheel (Chromium, WebKitGTK),
// or WebKit's own gesture events on a Mac. Each is kept from also zooming the
// page.

export interface GestureHandlers {
  /** The pointer moved this far while dragging, in CSS pixels. */
  turn(dx: number, dy: number): void;
  /** Zoom by `factor`: above 1 closer, below 1 farther. */
  zoom(factor: number): void;
}

type WheelLike = Pick<WheelEvent, "deltaY" | "deltaMode" | "ctrlKey" | "metaKey">;

/** How much a wheel event zooms. Lines and pages count as pixels; a trackpad
 *  pinch (Ctrl or ⌘ + wheel) sends small deltas, so each counts for more. */
export function wheelZoom(e: WheelLike): number {
  const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1;
  const rate = e.ctrlKey || e.metaKey ? 0.01 : 0.0015;
  return Math.exp(-e.deltaY * unit * rate);
}

/** A `gesturechange` event, which the DOM type definitions don't declare. */
type GestureLike = Event & { scale?: number };

/** Listen on `el` for drags, pinches and the wheel; returns the detach. */
export function attachGestures(el: HTMLElement, handlers: GestureHandlers): () => void {
  const pointers = new Map<number, { x: number; y: number }>();
  let spread = 0;
  let gestureScale = 1;
  let gesturing = false;

  /** How far apart the two touch points are. */
  const distance = () => {
    const [a, b] = [...pointers.values()];
    return Math.hypot(a.x - b.x, a.y - b.y);
  };
  const down = (e: PointerEvent) => {
    if (e.pointerType === "mouse" && e.button !== 0) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    // The press itself focuses the canvas (it has a tabindex), which keeps
    // the keyboard in the preview without showing the keyboard focus ring.
    el.setPointerCapture?.(e.pointerId);
    spread = pointers.size === 2 ? distance() : 0;
  };
  const move = (e: PointerEvent) => {
    const last = pointers.get(e.pointerId);
    if (!last) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.size === 1) handlers.turn(e.clientX - last.x, e.clientY - last.y);
    else if (pointers.size === 2) pinchTo(distance());
  };
  const pinchTo = (now: number) => {
    if (spread > 0 && now > 0) handlers.zoom(now / spread);
    spread = now;
  };
  const up = (e: PointerEvent) => {
    pointers.delete(e.pointerId);
    spread = pointers.size === 2 ? distance() : 0;
  };
  const wheel = (e: WheelEvent) => {
    e.preventDefault();
    if (!gesturing) handlers.zoom(wheelZoom(e));
  };
  const gestureStart = (e: GestureLike) => {
    e.preventDefault();
    gesturing = true;
    gestureScale = 1;
  };
  const gestureChange = (e: GestureLike) => {
    e.preventDefault();
    const scale = e.scale ?? 1;
    if (scale > 0) handlers.zoom(scale / gestureScale);
    gestureScale = scale;
  };
  const gestureEnd = (e: GestureLike) => {
    e.preventDefault();
    gesturing = false;
  };

  const listeners: Array<[string, (e: never) => void, AddEventListenerOptions?]> = [
    ["pointerdown", down],
    ["pointermove", move],
    ["pointerup", up],
    ["pointercancel", up],
    ["wheel", wheel, { passive: false }],
    ["gesturestart", gestureStart],
    ["gesturechange", gestureChange],
    ["gestureend", gestureEnd],
  ];
  for (const [type, listener, options] of listeners) el.addEventListener(type, listener as EventListener, options);
  return () => {
    for (const [type, listener] of listeners) el.removeEventListener(type, listener as EventListener);
  };
}

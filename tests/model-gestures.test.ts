// Turning and zooming the 3D preview with a pointer (src/model-gestures.ts):
// a drag turns, two touch points, the wheel (Ctrl/⌘ + wheel is a trackpad
// pinch) and WebKit's gesture events zoom, and none zooms the page.
import { beforeEach, expect, test, vi } from "vitest";
import { attachGestures, wheelZoom } from "../src/model-gestures";

let el: HTMLElement;
let turn: ReturnType<typeof vi.fn>;
let zoom: ReturnType<typeof vi.fn>;
let detach: () => void;

/** jsdom has no PointerEvent: a mouse event carrying its fields. */
function pointer(type: string, id: number, x: number, y: number, init: { pointerType?: string; button?: number } = {}): void {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button: init.button ?? 0 });
  Object.defineProperties(event, { pointerId: { value: id }, pointerType: { value: init.pointerType ?? "mouse" } });
  el.dispatchEvent(event);
}

function wheel(init: WheelEventInit): WheelEvent {
  const event = new WheelEvent("wheel", { bubbles: true, cancelable: true, ...init });
  el.dispatchEvent(event);
  return event;
}

function gesture(type: string, scale?: number): Event {
  const event = new Event(type, { bubbles: true, cancelable: true });
  if (scale !== undefined) Object.defineProperty(event, "scale", { value: scale });
  el.dispatchEvent(event);
  return event;
}

const factors = () => zoom.mock.calls.map(([f]) => Math.round((f as number) * 1000) / 1000);

beforeEach(() => {
  detach?.();
  el = document.createElement("canvas");
  el.tabIndex = -1;
  document.body.appendChild(el);
  turn = vi.fn();
  zoom = vi.fn();
  detach = attachGestures(el, { turn, zoom });
});

test("a left-button drag turns by how far the pointer moved", () => {
  pointer("pointerdown", 1, 100, 100);
  pointer("pointermove", 1, 130, 90);
  pointer("pointermove", 1, 135, 100);
  pointer("pointerup", 1, 135, 100);
  pointer("pointermove", 1, 200, 200);
  expect(turn.mock.calls).toEqual([[30, -10], [5, 10]]);
  expect(zoom).not.toHaveBeenCalled();
});

test("other mouse buttons don't turn it", () => {
  pointer("pointerdown", 1, 100, 100, { button: 2 });
  pointer("pointermove", 1, 150, 100);
  expect(turn).not.toHaveBeenCalled();
});

test("two touch points pinch: apart zooms in, together zooms out, and they don't turn it", () => {
  pointer("pointerdown", 1, 100, 100, { pointerType: "touch" });
  pointer("pointerdown", 2, 200, 100, { pointerType: "touch" });
  pointer("pointermove", 2, 300, 100, { pointerType: "touch" });
  pointer("pointermove", 2, 150, 100, { pointerType: "touch" });
  expect(factors()).toEqual([2, 0.25]);
  expect(turn).not.toHaveBeenCalled();
  // Lifting one finger leaves a one-finger drag, which turns again.
  pointer("pointercancel", 2, 150, 100, { pointerType: "touch" });
  pointer("pointermove", 1, 110, 100, { pointerType: "touch" });
  expect(turn.mock.calls).toEqual([[10, 0]]);
});

test("the wheel zooms, and is kept from scrolling or zooming the page", () => {
  const event = wheel({ deltaY: -100 });
  expect(event.defaultPrevented).toBe(true);
  wheel({ deltaY: 100 });
  const [closer, farther] = factors();
  expect(closer).toBeGreaterThan(1);
  expect(farther).toBeLessThan(1);
});

test("WebKit's pinch gestures zoom by the change in scale, and the wheel waits until they end", () => {
  expect(gesture("gesturestart").defaultPrevented).toBe(true);
  gesture("gesturechange", 1.5);
  gesture("gesturechange", 3);
  wheel({ deltaY: -10, ctrlKey: true });
  expect(gesture("gestureend").defaultPrevented).toBe(true);
  expect(factors()).toEqual([1.5, 2]);
  wheel({ deltaY: -10, ctrlKey: true });
  expect(zoom).toHaveBeenCalledTimes(3);
});

test("detaching stops listening", () => {
  detach();
  pointer("pointerdown", 1, 0, 0);
  pointer("pointermove", 1, 10, 0);
  wheel({ deltaY: -100 });
  expect(turn).not.toHaveBeenCalled();
  expect(zoom).not.toHaveBeenCalled();
});

test.each([
  ["a wheel notch in pixels", { deltaY: -100, deltaMode: 0, ctrlKey: false, metaKey: false }, 1.162],
  ["three lines", { deltaY: -3, deltaMode: 1, ctrlKey: false, metaKey: false }, 1.075],
  ["a page", { deltaY: 1, deltaMode: 2, ctrlKey: false, metaKey: false }, 0.549],
  ["a trackpad pinch (Ctrl + wheel)", { deltaY: -10, deltaMode: 0, ctrlKey: true, metaKey: false }, 1.105],
  ["a pinch on a Mac (⌘ + wheel)", { deltaY: 10, deltaMode: 0, ctrlKey: false, metaKey: true }, 0.905],
])("%s zooms by %s", (_name, e, factor) => {
  expect(Math.round(wheelZoom(e) * 1000) / 1000).toBe(factor);
});

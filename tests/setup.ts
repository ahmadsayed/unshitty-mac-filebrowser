// jsdom has no layout engine. Give the app the few browser APIs it needs so
// the real UI code runs: element sizes, scrolling, ResizeObserver, the
// clipboard and a Mac user agent (the app takes its macOS code paths).
import { vi } from "vitest";

const WIDTH = 1000;
const HEIGHT = 700;

Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", configurable: true });
for (const [prop, value] of [["clientWidth", WIDTH], ["clientHeight", HEIGHT], ["offsetWidth", 200], ["offsetHeight", 120]] as const) {
  Object.defineProperty(HTMLElement.prototype, prop, { get: () => value, configurable: true });
}
Object.defineProperty(window, "innerWidth", { value: WIDTH, configurable: true });
Object.defineProperty(window, "innerHeight", { value: HEIGHT, configurable: true });
Element.prototype.scrollTo = function () {} as typeof Element.prototype.scrollTo;
Element.prototype.scrollIntoView = function () {};
document.elementFromPoint = () => null;
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;
URL.createObjectURL = vi.fn(() => "blob:test");
URL.revokeObjectURL = vi.fn();
Object.defineProperty(navigator, "clipboard", {
  value: { writeText: vi.fn(async () => {}), readText: vi.fn(async () => "") },
  configurable: true,
});
// The mock backend waits 30 ms per call to feel real; tests don't need that.
(globalThis as Record<string, unknown>).__MOCK_DELAY = 0;

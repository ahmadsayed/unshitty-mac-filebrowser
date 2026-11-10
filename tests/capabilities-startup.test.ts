// The app reads what each kind of place allows once, at startup, without
// waiting for it (src/capabilities.ts): the first view needs no answer, since
// the built-in table says the same as the backend's.
import { expect, test } from "vitest";
import { boot, names } from "./app";

type MockCalls = { count(cmd: string): number; hold(cmd: string): void; release(cmd: string): void };
const mockCalls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;

test("startup asks for the table once and shows Home without waiting for it", async () => {
  await import("../dev/mock-tauri/core");
  mockCalls().hold("capabilities");
  await boot();
  expect(mockCalls().count("capabilities")).toBe(1);
  expect(names().length).toBeGreaterThan(0);
  mockCalls().release("capabilities");
});

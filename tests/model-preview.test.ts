// Space on a 3D model (glTF, GLB, STL, 3MF) shows it in the preview: the real
// app and loader against the mock, with only the WebGL drawing (showModel)
// replaced, since jsdom has none. Closing the preview releases the view.
import { beforeAll, beforeEach, expect, test, vi } from "vitest";
import { Group, type Object3D } from "three";
import { $, boot, click, goTo, hit, key, settle } from "./app";
import { SAMPLE_TRIANGLES } from "../dev/mock-tauri/models";
import { ARBESK_MOCK_ADDRESS, invoke } from "../dev/mock-tauri/core";

const viewer = vi.hoisted(() => ({
  showModel: vi.fn(),
  loadModel: vi.fn(),
  real: {
    showModel: null as unknown as (host: HTMLElement, object: unknown) => unknown,
    loadModel: null as unknown as (...args: unknown[]) => unknown,
  },
}));

vi.mock("../src/model-viewer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/model-viewer")>();
  viewer.real.showModel = actual.showModel;
  viewer.real.loadModel = actual.loadModel as (...args: unknown[]) => unknown;
  viewer.loadModel.mockImplementation(actual.loadModel);
  return { ...actual, loadModel: viewer.loadModel, showModel: viewer.showModel };
});

const PRINTS = "/Users/me/Documents/Projects/Prints";
type MockCalls = { count(cmd: string): number; failNext(cmd: string, error: string): void };
const calls = () => (globalThis as unknown as { __mockCalls: MockCalls }).__mockCalls;

/** The fake view: a canvas in the preview, and a dispose to count. */
function fakeView(host: HTMLElement) {
  const canvas = document.createElement("canvas");
  canvas.className = "preview-model";
  canvas.tabIndex = -1;
  host.appendChild(canvas);
  return { dispose: vi.fn(() => canvas.remove()) };
}

function triangles(object: Object3D): number {
  let count = 0;
  object.traverse((node) => {
    const geometry = (node as { geometry?: { index: { count: number } | null; getAttribute(n: string): { count: number } } }).geometry;
    if (geometry) count += (geometry.index?.count ?? geometry.getAttribute("position").count) / 3;
  });
  return count;
}

/** Wait for `ok`, checking every 10 ms: the viewer (three.js) loads on the
 *  first preview, which takes a while under the coverage run. */
async function until(ok: () => boolean, ms = 10_000): Promise<void> {
  const end = Date.now() + ms;
  while (!ok()) {
    if (Date.now() > end) throw new Error("timed out waiting for the preview");
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

async function preview(name: string): Promise<void> {
  await click(hit(name));
  await key(" ");
  await until(() => shown());
  await settle(10);
}

/** Whether the preview shows its content (or why there is none), not the loading state. */
function shown(): boolean {
  return [...$("#preview-body").children].some((child) => !child.classList.contains("preview-loading"));
}

beforeAll(async () => {
  await boot();
  await key("1", { mod: true });
  await goTo(PRINTS);
});

beforeEach(() => {
  viewer.showModel.mockReset();
  viewer.showModel.mockImplementation(fakeView);
});

test.each(["bracket.stl", "bracket-binary.stl", "box.3mf", "lamp.glb"])("Space shows %s as a 3D model", async (name) => {
  await preview(name);
  expect($("#preview-layer").hidden).toBe(false);
  expect($("#preview-name").textContent).toBe(name);
  expect($("#preview-body canvas.preview-model")).toBeTruthy();
  const [host, model] = viewer.showModel.mock.calls[0];
  expect(host).toBe($("#preview-body"));
  expect(triangles(model)).toBe(SAMPLE_TRIANGLES);

  const view = viewer.showModel.mock.results[0].value as { dispose: ReturnType<typeof vi.fn> };
  await key("Escape");
  await settle(10);
  expect($("#preview-layer").hidden).toBe(true);
  expect(view.dispose).toHaveBeenCalledTimes(1);
});

test.each(["Escape", " "])("%j on the model being turned closes the preview", async (name) => {
  await preview("bracket.stl");
  const canvas = $<HTMLCanvasElement>("#preview-body canvas.preview-model");
  // Dragging focuses the canvas, so the view no longer sees the keys.
  canvas.focus();
  await key(name, { target: canvas });
  await settle(10);
  expect($("#preview-layer").hidden).toBe(true);
  expect(document.activeElement).toBe($("#view"));
});

test("a .gltf reads the companion .bin it names next to it, and nothing else", async () => {
  viewer.loadModel.mockResolvedValueOnce(new Group());
  const before = calls().count("preview_bytes");
  await preview("lamp.gltf");
  expect(calls().count("preview_bytes") - before).toBe(2);
  const [format, , companions] = viewer.loadModel.mock.lastCall!;
  expect(format).toBe("gltf");
  expect([...(companions as Map<string, ArrayBuffer>).entries()].map(([uri, data]) => [uri, data.byteLength])).toEqual([
    ["lamp.bin", 168],
  ]);
  expect(viewer.showModel).toHaveBeenCalledTimes(1);
  await key("Escape");
});

test("a model over the size cap says it is too large, without reading it", async () => {
  await preview("scan.stl");
  expect($("#preview-body .preview-empty").textContent).toBe("This file is too large to preview.");
  expect(viewer.showModel).not.toHaveBeenCalled();
  await key("Escape");
});

test("without WebGL the preview says there is none", async () => {
  viewer.showModel.mockImplementation((host: HTMLElement, object: unknown) => viewer.real.showModel(host, object));
  await preview("bracket.stl");
  expect($("#preview-body .preview-empty").textContent).toBe("No preview available.");
  expect($("#preview-body canvas")).toBeNull();
  await key("Escape");
});

test("a model that can't be read says there is no preview", async () => {
  calls().failNext("preview_bytes", "Failed to read file: Input/output error (os error 5)");
  await preview("box.3mf");
  expect($("#preview-body .preview-empty").textContent).toBe("No preview available.");
  await key("Escape");
});

test("a model closed before it finished loading is never shown", async () => {
  let finish!: (object: Object3D) => void;
  viewer.loadModel.mockImplementationOnce(() => new Promise<Object3D>((resolve) => (finish = resolve)));
  await click(hit("lamp.glb"));
  await key(" ");
  await settle(20);
  await key("Escape");
  await settle(5);
  finish(new Group());
  await settle(10);
  expect(viewer.showModel).not.toHaveBeenCalled();
  expect($("#preview-layer").hidden).toBe(true);
});

// ---- While a preview loads ---------------------------------------------------

/** Hold the next model load until `release` is called. */
function holdNextLoad(): () => void {
  let release = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  viewer.loadModel.mockImplementationOnce(async (...args: unknown[]) => {
    await held;
    return (viewer.real.loadModel as (...a: unknown[]) => unknown)(...args);
  });
  return release;
}

test("a model still loading shows its icon, a spinner and Loading…", async () => {
  const release = holdNextLoad();
  await click(hit("bracket.stl"));
  await key(" ");
  await new Promise((resolve) => setTimeout(resolve, 300));
  const loading = $("#preview-body .preview-loading");
  expect(loading).toBeTruthy();
  expect(loading.querySelector(".preview-spinner")).toBeTruthy();
  expect(loading.querySelector(".preview-empty-icon")).toBeTruthy();
  expect(loading.textContent).toBe("Loading…");
  release();
  await until(() => !!$("#preview-body canvas.preview-model"));
  expect($("#preview-body .preview-loading")).toBeNull();
  await key("Escape");
});

test("a preview that loads at once never shows Loading…", async () => {
  // Warm: the viewer's chunk is loaded; this load answers at once.
  await preview("bracket.stl");
  await key("Escape");
  viewer.loadModel.mockResolvedValueOnce(new Group());
  let flashed = 0;
  const watch = new MutationObserver((records) => {
    for (const record of records) {
      flashed += [...record.addedNodes].filter((node) => (node as Element).classList?.contains("preview-loading")).length;
    }
  });
  watch.observe($("#preview-body"), { childList: true, subtree: true });
  await preview("bracket.stl");
  await new Promise((resolve) => setTimeout(resolve, 300));
  watch.disconnect();
  expect(flashed).toBe(0);
  await key("Escape");
});

test("a preview closed while loading leaves nothing behind", async () => {
  const release = holdNextLoad();
  await click(hit("bracket.stl"));
  await key(" ");
  await new Promise((resolve) => setTimeout(resolve, 300));
  expect($("#preview-body .preview-loading")).toBeTruthy();
  await key("Escape");
  release();
  await settle(20);
  expect($("#preview-layer").hidden).toBe(true);
  expect(viewer.showModel).not.toHaveBeenCalled();
  // The next preview starts clean: no loading state left from the last one.
  await preview("bracket.stl");
  expect($("#preview-body .preview-loading")).toBeNull();
  await key("Escape");
});

// ---- Remote items: Arbesk's models preview, other remote files don't ----

const ARBESK_SHAPES = `arbesk://${ARBESK_MOCK_ADDRESS}/1~Shapes`;

async function connectArbesk(): Promise<void> {
  const flow = await invoke<{ flow_id: string }>("arbesk_connect_start", { email: "me@example.com" });
  await invoke("arbesk_connect_finish", { flowId: flow.flow_id });
}

test.each(["Box.gltf", "Box.3mf"])("Space shows the Arbesk asset %s as a 3D model", async (name) => {
  await connectArbesk();
  await goTo(ARBESK_SHAPES);
  await preview(name);
  expect($("#preview-name").textContent).toBe(name);
  expect($("#preview-body canvas.preview-model")).toBeTruthy();
  const [, model] = viewer.showModel.mock.calls[0];
  expect(triangles(model)).toBe(SAMPLE_TRIANGLES);
  await key("Escape");
});

test("an Arbesk asset too large to preview says so", async () => {
  await goTo(ARBESK_SHAPES);
  await preview("Big.gltf");
  expect($("#preview-body .preview-empty").textContent).toBe("This file is too large to preview.");
  expect(viewer.showModel).not.toHaveBeenCalled();
  await key("Escape");
});

// An Arbesk scene: the mock's "Table" holds two copies of one model (the
// second tinted red, 3 to the right) and a 3MF part.
const ARBESK_UNTITLED = `arbesk://${ARBESK_MOCK_ADDRESS}/2~Untitled%20Collection`;

test("Space shows an Arbesk scene: each part loaded once, a copy per placement", async () => {
  await goTo(ARBESK_UNTITLED);
  const before = calls().count("arbesk_part");
  await preview("Table.gltf");
  expect(calls().count("arbesk_part") - before).toBe(2);
  expect(viewer.showModel).toHaveBeenCalledTimes(1);
  const [, scene] = viewer.showModel.mock.calls[0] as [HTMLElement, Group];
  expect(scene.children.map((copy) => copy.position.x)).toEqual([0, 3, 0]);
  expect(triangles(scene)).toBe(3 * SAMPLE_TRIANGLES);
  await key("Escape");
});

test("leaving a scene before its parts load asks for no more of them", async () => {
  await goTo(ARBESK_UNTITLED);
  const before = calls().count("arbesk_part");
  await click(hit("Table.gltf"));
  // Space, then Escape before anything has loaded.
  const press = (k: string) => $("#view").dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));
  press(" ");
  press("Escape");
  await settle(50);
  expect(calls().count("arbesk_part") - before).toBeLessThan(2);
  expect(viewer.showModel).not.toHaveBeenCalled();
});

test("a scene part that fails is left out; the rest shows", async () => {
  await goTo(ARBESK_UNTITLED);
  calls().failNext("arbesk_part", "Arbesk's storage could not be reached: Bad gateway");
  await preview("Table.gltf");
  const [, scene] = viewer.showModel.mock.calls[0] as [HTMLElement, Group];
  expect(scene.children).toHaveLength(1);
  await key("Escape");
});

test("without WebGL an Arbesk scene says there is none", async () => {
  viewer.showModel.mockImplementation((host: HTMLElement, object: unknown) => viewer.real.showModel(host, object));
  await goTo(ARBESK_UNTITLED);
  await preview("Table.gltf");
  expect($("#preview-body .preview-empty").textContent).toBe("No preview available.");
  expect($("#preview-body canvas")).toBeNull();
  await key("Escape");
});

test("an Arbesk asset whose only part fails has no preview", async () => {
  await goTo(ARBESK_UNTITLED);
  calls().failNext("arbesk_part", "Arbesk's storage could not be reached: Bad gateway");
  await preview("Cube.gltf");
  expect($("#preview-body .preview-empty").textContent).toBe("No preview available.");
  expect(viewer.showModel).not.toHaveBeenCalled();
  await key("Escape");
});

test("Drive and SSH files still have no preview", async () => {
  await invoke("drive_client_set", { json: JSON.stringify({ installed: { client_id: "1-a.apps.googleusercontent.com", client_secret: "s" } }) });
  const flow = await invoke<{ flow_id: string }>("drive_connect_start", {});
  await invoke("drive_connect_finish", { flowId: flow.flow_id });
  await invoke("sftp_connect", { url: "user@host" });
  for (const [where, name] of [["gdrive://mock/mock-root~My%20Drive", "hello.txt"], ["sftp://user@host/home/user", "notes.txt"]]) {
    // From Home: a typed address of another remote kind isn't taken inside Drive or SSH.
    await goTo("/Users/me");
    await goTo(where);
    await preview(name);
    expect($("#preview-body .preview-empty").textContent).toBe("Preview isn't available for remote items.");
    await key("Escape");
  }
});

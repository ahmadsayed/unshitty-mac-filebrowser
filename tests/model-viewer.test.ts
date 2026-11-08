// The 3D viewer's loading half (src/model-viewer.ts) reads real files: the
// mock's sample box as STL (ASCII and binary), 3MF, GLB and glTF. Drawing
// needs WebGL, which jsdom lacks, so showModel is checked to fail cleanly.
import { expect, test, vi } from "vitest";
import { Box3, BoxGeometry, Group, Mesh, Quaternion, Vector3, type Object3D } from "three";
import {
  dracoLoader, dragTurns, keyAction, loadModel, pickedPoint, recentred, resourceUrl, modelControls, shownBounds, showModel, turned, turnForKey,
  zoomed, zoomForKey, type Turn,
} from "../src/model-viewer";
import {
  SAMPLE_TRIANGLES, sample3mf, sampleGlb, sampleGltf, sampleGltfBin, sampleGltfJson, sampleStlAscii, sampleStlBinary,
} from "../dev/mock-tauri/models";

/** The bytes copied into this realm's ArrayBuffer (jsdom's TextEncoder answers in another one). */
const buffer = (bytes: Uint8Array): ArrayBuffer => {
  const out = new Uint8Array(bytes.length);
  out.set(bytes);
  return out.buffer;
};

function triangles(object: Object3D): number {
  let count = 0;
  object.traverse((node) => {
    if (!(node instanceof Mesh)) return;
    const geometry = node.geometry;
    count += (geometry.index?.count ?? geometry.getAttribute("position").count) / 3;
  });
  return count;
}

const size = (object: Object3D) => new Box3().setFromObject(object).getSize(new Vector3());

test.each([
  ["ASCII STL", () => loadModel("stl", buffer(sampleStlAscii()))],
  ["binary STL", () => loadModel("stl", buffer(sampleStlBinary()))],
  ["3MF", () => loadModel("3mf", buffer(sample3mf()))],
])("a %s box loads and stands up: printing formats are Z-up", async (_name, load) => {
  const model = await load();
  expect(triangles(model)).toBe(SAMPLE_TRIANGLES);
  // 40 wide, 20 deep, 10 tall in the file (Z up) is 10 tall in Y on screen.
  const { x, y, z } = size(model);
  expect([x, y, z].map(Math.round)).toEqual([40, 10, 20]);
});

/** Each mesh's material colour, as hex. */
function colours(object: Object3D): string[] {
  const found: string[] = [];
  object.traverse((node) => node instanceof Mesh && found.push((node.material as { color: { getHexString(): string } }).color.getHexString()));
  return found;
}

test("a model that names no colour is drawn grey; one that does keeps it", async () => {
  expect(colours(await loadModel("3mf", buffer(sample3mf())))).toEqual(["8c8c96"]);
  expect(colours(await loadModel("stl", buffer(sampleStlAscii())))).toEqual(["8c8c96"]);
  const json = sampleGltfJson(`data:application/octet-stream;base64,${Buffer.from(sampleGltfBin()).toString("base64")}`);
  delete json.materials;
  (json.meshes as Array<{ primitives: Array<{ material?: number }> }>)[0].primitives[0].material = undefined;
  expect(colours(await loadModel("gltf", buffer(new TextEncoder().encode(JSON.stringify(json)))))).toEqual(["8c8c96"]);
  // The sample GLB's own blue survives (linear 0.35, 0.55, 0.85 as sRGB).
  expect(colours(await loadModel("glb", buffer(sampleGlb())))).not.toEqual(["8c8c96"]);
});

test("a GLB box loads as it is: glTF is already Y-up", async () => {
  const model = await loadModel("glb", buffer(sampleGlb()));
  expect(triangles(model)).toBe(SAMPLE_TRIANGLES);
  const { x, y, z } = size(model);
  expect([x, y, z].map(Math.round)).toEqual([40, 20, 10]);
});

test("a .gltf with its buffer embedded as a data URI loads without companions", async () => {
  const base64 = Buffer.from(sampleGltfBin()).toString("base64");
  const json = sampleGltfJson(`data:application/octet-stream;base64,${base64}`);
  const model = await loadModel("gltf", buffer(new TextEncoder().encode(JSON.stringify(json))));
  expect(triangles(model)).toBe(SAMPLE_TRIANGLES);
});

test("a .gltf loads its companion .bin from the bytes it is given, and lets the URL go", async () => {
  // jsdom's blob URLs can't be fetched; hand the loader the same bytes as a data URI.
  const bin = `data:application/octet-stream;base64,${Buffer.from(sampleGltfBin()).toString("base64")}`;
  const create = vi.spyOn(URL, "createObjectURL").mockReturnValue(bin);
  const revoke = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
  try {
    const model = await loadModel("gltf", buffer(sampleGltf("lamp.bin")), new Map([["lamp.bin", buffer(sampleGltfBin())]]));
    expect(triangles(model)).toBe(SAMPLE_TRIANGLES);
    expect(create).toHaveBeenCalledTimes(1);
    expect(revoke).toHaveBeenCalledWith(bin);
  } finally {
    create.mockRestore();
    revoke.mockRestore();
  }
});

test("a .gltf whose companion wasn't read fails instead of fetching it", async () => {
  await expect(loadModel("gltf", buffer(sampleGltf("lamp.bin")))).rejects.toThrow();
});

test("files that hold no model are refused", async () => {
  const empty = JSON.stringify({ asset: { version: "2.0" }, scene: 0, scenes: [{ nodes: [] }] });
  await expect(loadModel("gltf", buffer(new TextEncoder().encode(empty)))).rejects.toThrow("nothing to show");
  await expect(loadModel("glb", new ArrayBuffer(12))).rejects.toThrow();
  await expect(loadModel("3mf", new ArrayBuffer(12))).rejects.toThrow();
});

test("without WebGL, showModel throws and leaves nothing behind", async () => {
  const host = document.createElement("div");
  const model = await loadModel("stl", buffer(sampleStlAscii()));
  expect(() => showModel(host, model)).toThrow();
  expect(host.querySelector("canvas")).toBeNull();
});

// ----- Turning the model -----

const RIGHT = new Vector3(1, 0, 0);
/** Where the model's front (facing the viewer, +Z) points after `goal`. */
const front = (goal: Quaternion) => new Vector3(0, 0, 1).applyQuaternion(goal).toArray().map((v) => Math.round(v * 1000) / 1000 + 0);

test("Rotate Left and Right swing the front to that side, a quarter turn about the vertical", () => {
  const [left, right] = (() => {
    const asked: Turn[] = [];
    const bar = modelControls((t) => asked.push(t));
    for (const button of bar.querySelectorAll("button")) button.click();
    return asked;
  })();
  expect(front(turned(new Quaternion(), left, RIGHT))).toEqual([-1, 0, 0]);
  expect(front(turned(new Quaternion(), right, RIGHT))).toEqual([1, 0, 0]);
});

test("the controls are Rotate Left, Rotate Right and Reset View, each a labelled button", () => {
  const asked: Turn[] = [];
  const bar = modelControls((t) => asked.push(t));
  const buttons = [...bar.querySelectorAll("button")];
  expect(buttons.map((b) => [b.title, b.getAttribute("aria-label")])).toEqual([
    ["Rotate Left", "Rotate Left"],
    ["Rotate Right", "Rotate Right"],
    ["Reset View", "Reset View"],
  ]);
  buttons[2].click();
  expect(asked).toEqual(["reset"]);
});

test("arrow keys turn in 15° steps the way a drag would, Home resets, other keys do nothing", () => {
  expect(front(turned(new Quaternion(), turnForKey("ArrowLeft")!, RIGHT))[0]).toBeLessThan(0);
  expect(front(turned(new Quaternion(), turnForKey("ArrowRight")!, RIGHT))[0]).toBeGreaterThan(0);
  // Up tips the front up, showing the underside, as dragging up does.
  expect(front(turned(new Quaternion(), turnForKey("ArrowUp")!, RIGHT))[1]).toBeGreaterThan(0);
  expect(front(turned(new Quaternion(), turnForKey("ArrowDown")!, RIGHT))[1]).toBeLessThan(0);
  expect(turnForKey("Home")).toBe("reset");
  expect(turnForKey("a")).toBeNull();
  expect(turnForKey("Escape")).toBeNull();
});

test("turns add up, pitch follows the view's own horizontal axis, and reset undoes them all", () => {
  const step = turnForKey("ArrowRight")!;
  let goal = new Quaternion();
  for (let i = 0; i < 6; i++) goal = turned(goal, step, RIGHT);
  expect(front(goal)).toEqual([1, 0, 0]);
  // Seen from the side (the view's right is world -Z), pitch turns about that axis.
  expect(front(turned(new Quaternion(), { axis: "pitch", degrees: -90 }, new Vector3(0, 0, -2)))).toEqual([0, 0, 1]);
  expect(turned(goal, "reset", RIGHT).equals(new Quaternion())).toBe(true);
});

test("a drag turns the model's front after the pointer, a whole turn across the canvas", () => {
  const turnAll = (turns: Turn[]) => turns.reduce((goal, t) => turned(goal, t, RIGHT), new Quaternion());
  // Half the width to the right: half a turn, the front now facing away.
  expect(front(turnAll(dragTurns(300, 0, 600)))).toEqual([0, 0, -1]);
  // A quarter of the width to the right swings the front right; down tips it down.
  expect(front(turnAll(dragTurns(150, 0, 600)))).toEqual([1, 0, 0]);
  expect(front(turnAll(dragTurns(0, 150, 600)))).toEqual([0, -1, 0]);
  expect(dragTurns(10, 0, 0)[0]).toEqual({ axis: "yaw", degrees: 3600 });
});

test("zoom keeps the camera between close up and far away", () => {
  expect(zoomed(100, 2, 50)).toBe(50);
  expect(zoomed(100, 0.5, 50)).toBe(200);
  expect(zoomed(20, 4, 50)).toBe(15);
  expect(zoomed(500, 0.01, 50)).toBe(600);
});

test("keys on the model: arrows and Home turn, + and - zoom, and a modifier leaves them to the app", () => {
  const key = (k: string, mods: Partial<Record<"ctrlKey" | "metaKey" | "altKey", boolean>> = {}) =>
    keyAction({ key: k, ctrlKey: false, metaKey: false, altKey: false, ...mods });
  expect(key("ArrowLeft")).toEqual({ axis: "yaw", degrees: -15 });
  expect(key("Home")).toBe("reset");
  expect([key("+"), key("="), key("-")]).toEqual([1.25, 1.25, 0.8]);
  expect(key("x")).toBeNull();
  expect(key("+", { ctrlKey: true })).toBeNull();
  expect(key("ArrowLeft", { metaKey: true })).toBeNull();
  expect(key("Home", { altKey: true })).toBeNull();
  expect(zoomForKey("Enter")).toBeNull();
});

test("a glTF may fetch its companions, its own data URIs and the blob URLs made for its embedded images, nothing else", () => {
  const urls = new Map([["lamp.bin", "blob:app/companion"]]);
  expect(resourceUrl("lamp.bin", urls)).toBe("blob:app/companion");
  expect(resourceUrl("data:image/png;base64,iVBOR", urls)).toBe("data:image/png;base64,iVBOR");
  // GLTFLoader turns an image stored in the file into a blob URL itself.
  expect(resourceUrl("blob:http://localhost/1d852c0a", urls)).toBe("blob:http://localhost/1d852c0a");
  for (const other of ["textures/wood.png", "https://example.com/a.bin", "file:///etc/passwd", "/etc/passwd"]) {
    expect(resourceUrl(other, urls)).toBe("data:application/octet-stream;base64,");
  }
});

test("compressed meshes decode with three's glTF Draco build, one decoder for every preview", () => {
  const draco = dracoLoader();
  expect(dracoLoader()).toBe(draco);
  const paths = (draco as unknown as { decoderPaths: Record<string, string | null> }).decoderPaths;
  expect(paths.wasm).toMatch(/libs\/draco\/gltf\/draco_decoder\.wasm$/);
  expect(paths.js).toMatch(/libs\/draco\/gltf\/draco_wasm_wrapper\.js$/);
  expect(paths.dep_js).toBeNull();
  expect((draco as unknown as { workerLimit: number }).workerLimit).toBe(2);
});

// ----- Where the model turns about -----

const round = (v: Vector3) => v.toArray().map((x) => Math.round(x * 1000) / 1000 + 0);

test("the pivot is the middle of what the model shows: its vertices, not its stored bounds", () => {
  // A 2-unit cube whose stored bounds claim it reaches 10 units up, as
  // geometry bounds after compression or from an exporter can.
  const geometry = new BoxGeometry(2, 2, 2);
  geometry.computeBoundingBox();
  geometry.boundingBox!.max.y = 10;
  const model = new Group().add(new Mesh(geometry));
  expect(round(new Box3().setFromObject(model).getCenter(new Vector3()))).toEqual([0, 4.5, 0]);
  const shown = shownBounds(model);
  expect(round(shown.getCenter(new Vector3()))).toEqual([0, 0, 0]);
  expect(round(shown.getSize(new Vector3()))).toEqual([2, 2, 2]);
});

test("hidden meshes don't count, and moved or turned parts count where they are", () => {
  const model = new Group();
  const part = new Mesh(new BoxGeometry(2, 2, 2));
  part.position.set(10, 0, 0);
  const hidden = new Mesh(new BoxGeometry(100, 100, 100));
  hidden.visible = false;
  const hiddenGroup = new Group().add(new Mesh(new BoxGeometry(50, 50, 50)));
  hiddenGroup.visible = false;
  model.add(part, hidden, hiddenGroup);
  model.rotation.y = Math.PI / 2;
  const shown = shownBounds(model);
  // Turned a quarter about the vertical, the part at x = 10 is at z = -10.
  expect(round(shown.getCenter(new Vector3()))).toEqual([0, 0, -10]);
  expect(round(shown.getSize(new Vector3()))).toEqual([2, 2, 2]);
});

test("a model with nothing visible shows an empty box (centring then falls back to its stored bounds)", () => {
  const mesh = new Mesh(new BoxGeometry(2, 2, 2));
  mesh.visible = false;
  const model = new Group().add(mesh);
  expect(shownBounds(model).isEmpty()).toBe(true);
});

test("a double-clicked point becomes the centre: moving the model there brings that point to the pivot", () => {
  // The model sits in a pivot at the origin, turned a quarter about the vertical.
  const pivot = new Group();
  const model = new Group();
  model.position.set(1, 2, 3);
  pivot.add(model);
  pivot.quaternion.setFromAxisAngle(new Vector3(0, 1, 0), Math.PI / 2);
  pivot.updateMatrixWorld(true);
  // A point on the model, as the double-click's ray hit it, in world space.
  const onModel = new Vector3(4, -1, 2);
  const hit = model.localToWorld(onModel.clone());
  model.position.copy(recentred(model.position, pivot.quaternion, hit));
  pivot.updateMatrixWorld(true);
  expect(round(model.localToWorld(onModel.clone()))).toEqual([0, 0, 0]);
});

test("recentring keeps the model's turn and leaves its inputs alone", () => {
  const position = new Vector3(1, 1, 1);
  const turn = new Quaternion().setFromAxisAngle(new Vector3(1, 0, 0), 0.3);
  const point = new Vector3(0, 0, 0);
  expect(round(recentred(position, turn, point))).toEqual([1, 1, 1]);
  expect(round(position)).toEqual([1, 1, 1]);
  expect(turn.equals(new Quaternion().setFromAxisAngle(new Vector3(1, 0, 0), 0.3))).toBe(true);
});

/** A scene 20 units across: a floor, and a small figure made of a body and a hat. */
function scene() {
  const root = new Group();
  const floor = new Mesh(new BoxGeometry(20, 0.1, 20));
  const figure = new Group();
  figure.position.set(5, 0, -3);
  const body = new Mesh(new BoxGeometry(1, 1, 1));
  body.position.y = 0.5;
  const hat = new Mesh(new BoxGeometry(1, 0.4, 1));
  hat.position.y = 1.2;
  figure.add(body, hat);
  root.add(floor, figure);
  root.updateMatrixWorld(true);
  const size = shownBounds(root).getSize(new Vector3()).length();
  return { root, floor, figure, body, hat, size };
}

test("double-clicking part of a small object turns the model about the whole object, so it spins in place", () => {
  const { root, body, hat, size } = scene();
  const hitOnBody = new Vector3(5, 0.5, -2.5);
  // The body and the hat belong to one figure: its middle, from its foot to the top of its hat.
  expect(round(pickedPoint(hitOnBody, body, root, size))).toEqual([5, 0.7, -3]);
  expect(round(pickedPoint(new Vector3(5, 1.3, -2.5), hat, root, size))).toEqual([5, 0.7, -3]);
});

test("double-clicking something as large as the scene keeps the point that was hit", () => {
  const { root, floor, size } = scene();
  // The floor's middle may be far from where you clicked.
  const hit = new Vector3(7, 0.05, -6);
  expect(round(pickedPoint(hit, floor, root, size))).toEqual([7, 0.05, -6]);
});

test("a whole single-part model picks its own middle only when it is small beside the model, which it never is", () => {
  const model = new Group().add(new Mesh(new BoxGeometry(2, 2, 2)));
  model.updateMatrixWorld(true);
  const size = shownBounds(model).getSize(new Vector3()).length();
  const hit = new Vector3(0.3, 1, 0.2);
  expect(round(pickedPoint(hit, model.children[0], model, size))).toEqual([0.3, 1, 0.2]);
});

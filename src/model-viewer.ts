// The Space preview's 3D viewer, built on three.js and loaded only when a
// model is previewed, so the app starts without it. `loadModel` turns a
// file's bytes into a three.js object (no WebGL needed, so it is tested);
// `showModel` draws it on a canvas you turn by dragging (src/model-gestures.ts),
// with the buttons over it or the arrow keys, and zoom with the wheel, a
// pinch or + and -; its `dispose`
// gives the GPU memory back when the preview closes.

import {
  Box3, DirectionalLight, Group, HemisphereLight, Loader, LoadingManager, Mesh, MeshStandardMaterial, NeutralToneMapping,
  PerspectiveCamera, PMREMGenerator, Quaternion, Raycaster, Scene, Texture, Vector2, Vector3, WebGLRenderer, type BufferGeometry,
  type Material, type Object3D,
} from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { DRACO_GLTF_CONFIG, DRACOLoader } from "three/addons/loaders/DRACOLoader.js";
import { STLLoader } from "three/addons/loaders/STLLoader.js";
import { ThreeMFLoader } from "three/addons/loaders/3MFLoader.js";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";
import type { ModelFormat } from "./models";
import { sym } from "./icons";
import { attachGestures } from "./model-gestures";

/** A model on screen. */
export interface ModelView {
  dispose(): void;
}

/** Companion files a .gltf names, by the URI it names them with. */
export type Companions = Map<string, ArrayBuffer>;

/** STL and 3MF are Z-up (3D printing); three.js is Y-up. */
function standUp(object: Object3D): Object3D {
  object.rotation.x = -Math.PI / 2;
  return object;
}

/** What a model with no colour of its own is drawn in: a mid grey reads on
 *  the light and the dark preview alike. */
const neutral = (coloured = false) =>
  new MeshStandardMaterial({ color: coloured ? 0xffffff : 0x8c8c96, vertexColors: coloured, roughness: 0.55, metalness: 0.05 });

function stlMesh(geometry: BufferGeometry): Mesh {
  return new Mesh(geometry, neutral(geometry.hasAttribute("color")));
}

/** The loaders' stand-in for a file that names no material (pure white, or
 *  white metal for glTF) becomes the neutral grey; a model's own colours stay. */
function neutralDefaults(object: Object3D): void {
  object.traverse((node) => {
    if (!(node instanceof Mesh)) return;
    const swap = (material: Material) => (material.name === Loader.DEFAULT_MATERIAL_NAME ? neutral() : material);
    node.material = Array.isArray(node.material) ? node.material.map(swap) : swap(node.material);
  });
}

/**
 * The Draco decoder for compressed glTF meshes (KHR_draco_mesh_compression,
 * what Blender's exporter writes when compression is on). Its glTF-only
 * WebAssembly build ships with three.js and is bundled with the viewer; one
 * decoder, made on first use, serves every preview, so its workers (two at
 * most) and WebAssembly load once.
 */
let draco: DRACOLoader | null = null;
export function dracoLoader(): DRACOLoader {
  draco ??= new DRACOLoader().setDecoderPath(DRACO_GLTF_CONFIG).setWorkerLimit(2);
  return draco;
}

/** An empty resource, for a file a glTF names that wasn't read for it. */
const NOTHING = "data:application/octet-stream;base64,";

/**
 * The URL the glTF loader may fetch for `url`: a companion read for it (its
 * blob URL in `urls`), or a URL that is the file's own bytes (an embedded
 * data URI, or the blob URL the loader makes for an image stored in the
 * file). Anything else is empty, so nothing outside the file is fetched.
 */
export function resourceUrl(url: string, urls: ReadonlyMap<string, string>): string {
  return urls.get(url) ?? (url.startsWith("data:") || url.startsWith("blob:") ? url : NOTHING);
}

/** Read a glTF; the companion files it names come from `companions`, and
 *  nothing else is fetched. */
async function gltfScene(bytes: ArrayBuffer, companions: Companions): Promise<Object3D> {
  const urls = new Map([...companions].map(([uri, data]) => [uri, URL.createObjectURL(new Blob([data]))]));
  const manager = new LoadingManager();
  manager.setURLModifier((url) => resourceUrl(url, urls));
  try {
    const gltf = await new GLTFLoader(manager).setDRACOLoader(dracoLoader()).parseAsync(bytes, "");
    // The loader's stand-in for a primitive with no material maps to none of
    // the file's (its flat-shaded copy is stored with an undefined mapping).
    gltf.scene.traverse((node) => {
      if (node instanceof Mesh && !Array.isArray(node.material) && !gltf.parser.associations.get(node.material)) {
        node.material.name = Loader.DEFAULT_MATERIAL_NAME;
      }
    });
    return gltf.scene;
  } finally {
    urls.forEach((url) => URL.revokeObjectURL(url));
  }
}

const LOADERS: Record<ModelFormat, (bytes: ArrayBuffer, companions: Companions) => Promise<Object3D>> = {
  stl: async (bytes) => standUp(stlMesh(new STLLoader().parse(bytes))),
  "3mf": async (bytes) => standUp(new ThreeMFLoader().parse(bytes)),
  glb: (bytes) => gltfScene(bytes, new Map()),
  gltf: gltfScene,
};

/** The model in `bytes`, or an error when it holds nothing to show. */
export async function loadModel(format: ModelFormat, bytes: ArrayBuffer, companions: Companions = new Map()): Promise<Object3D> {
  const object = await LOADERS[format](bytes, companions);
  neutralDefaults(object);
  object.updateMatrixWorld(true);
  if (new Box3().setFromObject(object).isEmpty()) throw new Error("The model has nothing to show.");
  return object;
}

/** The most vertices `shownBounds` reads; a bigger model is sampled evenly. */
const BOUNDS_SAMPLES = 400_000;

/**
 * The box around what the model shows: the visible meshes' own vertices
 * where they are, not the bounds their geometry stores, which can be larger
 * (an exporter's or a compressed mesh's), and nothing hidden. A model of
 * millions of vertices is sampled; for a pivot that is close enough.
 */
export function shownBounds(object: Object3D): Box3 {
  object.updateMatrixWorld(true);
  const meshes: Mesh[] = [];
  object.traverseVisible((node) => node instanceof Mesh && meshes.push(node));
  const total = meshes.reduce((sum, mesh) => sum + mesh.geometry.getAttribute("position").count, 0);
  const stride = Math.max(1, Math.ceil(total / BOUNDS_SAMPLES));
  const box = new Box3();
  const point = new Vector3();
  for (const mesh of meshes) {
    const count = mesh.geometry.getAttribute("position").count;
    for (let i = 0; i < count; i += stride) box.expandByPoint(mesh.getVertexPosition(i, point).applyMatrix4(mesh.matrixWorld));
  }
  return box;
}

/** Centre the model at the origin, inside a pivot, and say how big it is.
 *  It turns about the middle of what it shows. */
function centred(object: Object3D): { pivot: Group; size: number } {
  const shown = shownBounds(object);
  const box = shown.isEmpty() ? new Box3().setFromObject(object) : shown;
  object.position.sub(box.getCenter(new Vector3()));
  const pivot = new Group();
  pivot.add(object);
  return { pivot, size: box.getSize(new Vector3()).length() || 1 };
}

/**
 * Where the model goes so that it turns about `point` (world space, where a
 * double-click's ray hit it), with that point at the centre of the view: the
 * pivot sits at the origin, so the point's place in the pivot's own frame
 * (undoing the pivot's turn `pivot`) is taken off the model's `position`.
 */
export function recentred(position: Vector3, pivot: Quaternion, point: Vector3): Vector3 {
  return position.clone().sub(point.clone().applyQuaternion(pivot.clone().invert()));
}

/** Moves `object` towards a position over TURN_MS (at once with reduced
 *  motion), rendering every frame. */
function glider(object: Object3D, render: () => void) {
  let frame = 0;
  return {
    to(goal: Vector3) {
      cancelAnimationFrame(frame);
      const from = object.position.clone();
      const start = performance.now();
      const step = (now: number) => {
        const still = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
        const t = still ? 1 : Math.min(1, (now - start) / TURN_MS);
        object.position.lerpVectors(from, goal, ease(t));
        render();
        if (t < 1) frame = requestAnimationFrame(step);
      };
      frame = requestAnimationFrame(step);
    },
    stop: () => cancelAnimationFrame(frame),
  };
}

/** Where the camera looks from: the front, a little above and to the right.
 *  It only ever moves along this line; the model turns instead. */
const VIEW_DIRECTION = new Vector3(0.55, 0.45, 1).normalize();

/** How far away the whole of a model `size` across fills the view. */
function fitDistance(camera: PerspectiveCamera, size: number): number {
  return (size / 2 / Math.tan((camera.fov * Math.PI) / 360)) * 1.15;
}

/** How close and how far the camera may go, as multiples of the model's size. */
const NEAREST = 0.3;
const FARTHEST = 12;

/** The camera's distance after zooming by `factor` (above 1 closer), kept
 *  within reach of a model `size` across. */
export function zoomed(distance: number, factor: number, size: number): number {
  return Math.min(size * FARTHEST, Math.max(size * NEAREST, distance / factor));
}

/** Free a scene's geometries, materials and textures. */
function disposeScene(scene: Scene): void {
  scene.traverse((node) => {
    if (!(node instanceof Mesh)) return;
    node.geometry.dispose();
    const materials: Material[] = Array.isArray(node.material) ? node.material : [node.material];
    for (const material of materials) {
      Object.values(material).forEach((value) => value instanceof Texture && value.dispose());
      material.dispose();
    }
  });
}

// ---------------------------------------------------------------------------
// Turning the model: buttons over it and arrow keys on it
// ---------------------------------------------------------------------------

/** A turn of the model: degrees about the vertical axis (yaw) or the view's
 *  horizontal one (pitch), or back to how it first showed. */
export type Turn = { axis: "yaw" | "pitch"; degrees: number } | "reset";

/** Arrow keys turn the model in steps, its front following the arrow as it
 *  follows a drag; Home puts it back. */
const KEY_TURNS: Record<string, Turn> = {
  ArrowLeft: { axis: "yaw", degrees: -15 },
  ArrowRight: { axis: "yaw", degrees: 15 },
  ArrowUp: { axis: "pitch", degrees: -15 },
  ArrowDown: { axis: "pitch", degrees: 15 },
  Home: "reset",
};

export function turnForKey(key: string): Turn | null {
  return KEY_TURNS[key] ?? null;
}

/** + and - zoom in and out a step, as the wheel does. */
const KEY_ZOOMS: Record<string, number> = { "+": 1.25, "=": 1.25, "-": 0.8 };

export function zoomForKey(key: string): number | null {
  return KEY_ZOOMS[key] ?? null;
}

/** What a key on the model does: a turn, a zoom factor, or nothing (with a
 *  modifier held it is one of the app's own shortcuts). */
export function keyAction(e: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey">): Turn | number | null {
  if (e.ctrlKey || e.metaKey || e.altKey) return null;
  return turnForKey(e.key) ?? zoomForKey(e.key);
}

/** The turns for a drag of (dx, dy) px on a canvas `width` px wide: a whole
 *  turn across it. The front follows the pointer: right swings it right
 *  about the vertical, down tips it down. */
export function dragTurns(dx: number, dy: number, width: number): Turn[] {
  const perPixel = 360 / Math.max(width, 1);
  return [
    { axis: "yaw", degrees: dx * perPixel },
    { axis: "pitch", degrees: dy * perPixel },
  ];
}

/** The buttons over the model, as GNOME's Image Viewer has for pictures. */
const BUTTONS: Array<{ icon: string; label: string; turn: Turn }> = [
  { icon: "object-rotate-left", label: "Rotate Left", turn: { axis: "yaw", degrees: -90 } },
  { icon: "object-rotate-right", label: "Rotate Right", turn: { axis: "yaw", degrees: 90 } },
  { icon: "view-reset", label: "Reset View", turn: "reset" },
];

/** The bar of turn buttons; each asks `turn` for its turn. */
export function modelControls(turn: (t: Turn) => void): HTMLElement {
  const bar = document.createElement("div");
  bar.className = "preview-model-controls";
  for (const { icon, label, turn: t } of BUTTONS) {
    const button = document.createElement("button");
    button.className = "flat-btn";
    button.title = label;
    button.setAttribute("aria-label", label);
    button.innerHTML = sym(icon);
    button.addEventListener("click", () => turn(t));
    bar.appendChild(button);
  }
  return bar;
}

/** Where `goal` ends up after `turn`: yaw about the world's up, pitch about
 *  `right` (the view's horizontal axis), reset back to no rotation. */
export function turned(goal: Quaternion, turn: Turn, right: Vector3): Quaternion {
  if (turn === "reset") return new Quaternion();
  const axis = turn.axis === "yaw" ? new Vector3(0, 1, 0) : right.clone().normalize();
  const step = new Quaternion().setFromAxisAngle(axis, (turn.degrees * Math.PI) / 180);
  return goal.clone().premultiply(step);
}

const TURN_MS = 200;
const ease = (t: number) => 1 - (1 - t) ** 3;

/** Turns `pivot` towards each goal over TURN_MS (at once when the user
 *  prefers reduced motion), rendering every frame. */
function turner(pivot: Object3D, render: () => void) {
  let goal = new Quaternion();
  let frame = 0;
  const animate = (from: Quaternion, start: number) => (now: number) => {
    const still = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    const t = still ? 1 : Math.min(1, (now - start) / TURN_MS);
    pivot.quaternion.slerpQuaternions(from, goal, ease(t));
    render();
    if (t < 1) frame = requestAnimationFrame(animate(from, start));
  };
  return {
    turn(t: Turn, right: Vector3) {
      goal = turned(goal, t, right);
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(animate(pivot.quaternion.clone(), performance.now()));
    },
    /** Turn at once, as a drag does, so the model never lags the pointer. */
    nudge(t: Turn, right: Vector3) {
      cancelAnimationFrame(frame);
      goal = turned(goal, t, right);
      pivot.quaternion.copy(goal);
    },
    stop: () => cancelAnimationFrame(frame),
  };
}

/**
 * Draw `object` in `host`, turned with the pointer, the buttons over it or the
 * arrow keys, and zoomed with the wheel.
 * It renders only when the view changes. Throws when WebGL is unavailable.
 */
export function showModel(host: HTMLElement, object: Object3D): ModelView {
  const renderer = new WebGLRenderer({ antialias: true, alpha: true });
  renderer.setPixelRatio(window.devicePixelRatio);
  // Keeps light surfaces from burning out to the background's white.
  renderer.toneMapping = NeutralToneMapping;
  const canvas = renderer.domElement;
  canvas.className = "preview-model";
  // Focusable, so a drag keeps the keyboard in the preview (Escape, Space).
  canvas.tabIndex = -1;
  canvas.setAttribute(
    "aria-label",
    "3D model: drag or use the arrow keys to turn; scroll, pinch or use + and - to zoom; double-click a point to turn around it",
  );
  const wrap = document.createElement("div");
  wrap.className = "preview-model-wrap";
  wrap.appendChild(canvas);
  host.appendChild(wrap);

  const scene = new Scene();
  const pmrem = new PMREMGenerator(renderer);
  scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
  scene.environmentIntensity = 0.5;
  pmrem.dispose();
  const { pivot, size } = centred(object);
  scene.add(pivot, new HemisphereLight(0xffffff, 0x50505a, 0.7));
  const camera = new PerspectiveCamera(35, 1, size / 100, size * 100);
  // Up and to the left of the eye, so the faces you see are shaded apart.
  const key = new DirectionalLight(0xffffff, 1.6);
  key.position.set(-1, 1.5, 0.5);
  camera.add(key);
  scene.add(camera);

  const render = () => renderer.render(scene, camera);
  const home = fitDistance(camera, size);
  let distance = home;
  const place = () => {
    camera.position.copy(VIEW_DIRECTION).multiplyScalar(distance);
    camera.lookAt(0, 0, 0);
    camera.updateMatrixWorld();
    render();
  };
  // Pitch tips the model about the view's own horizontal axis.
  const right = () => new Vector3().setFromMatrixColumn(camera.matrixWorld, 0);
  const turns = turner(pivot, render);
  const middle = object.position.clone();
  const glide = glider(object, render);
  const turn = (t: Turn) => {
    turns.turn(t, right());
    if (t === "reset") {
      distance = home;
      glide.to(middle);
      place();
    }
  };
  // A double-click turns the model about the point under the pointer from
  // then on, that point gliding to the centre; on empty space, about its
  // middle again.
  const raycaster = new Raycaster();
  const pick = (x: number, y: number) => {
    pivot.updateMatrixWorld(true);
    raycaster.setFromCamera(new Vector2((x / (canvas.clientWidth || 1)) * 2 - 1, 1 - (y / (canvas.clientHeight || 1)) * 2), camera);
    const hit = raycaster.intersectObject(pivot, true)[0];
    glide.to(hit ? recentred(object.position, pivot.quaternion, hit.point) : middle);
  };
  const zoom = (factor: number) => {
    distance = zoomed(distance, factor, size);
    place();
  };
  const detach = attachGestures(canvas, {
    turn: (dx, dy) => {
      for (const t of dragTurns(dx, dy, canvas.clientWidth)) turns.nudge(t, right());
      render();
    },
    zoom,
    pick,
  });
  wrap.appendChild(modelControls(turn));
  canvas.addEventListener("keydown", (event) => {
    const action = keyAction(event);
    if (action === null) return;
    event.preventDefault();
    event.stopPropagation();
    if (typeof action === "number") zoom(action);
    else turn(action);
  });

  const resize = () => {
    const width = canvas.clientWidth || 1;
    const height = canvas.clientHeight || 1;
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
    place();
  };
  const observer = new ResizeObserver(resize);
  observer.observe(canvas);
  resize();

  return {
    dispose() {
      detach();
      turns.stop();
      glide.stop();
      observer.disconnect();
      disposeScene(scene);
      scene.environment?.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
      wrap.remove();
    },
  };
}

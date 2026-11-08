// The Space preview's 3D viewer, built on three.js and loaded only when a
// model is previewed, so the app starts without it. `loadModel` turns a
// file's bytes into a three.js object (no WebGL needed, so it is tested);
// `showModel` draws it on a canvas you can turn and zoom, and its `dispose`
// gives the GPU memory back when the preview closes.

import {
  Box3, DirectionalLight, Group, HemisphereLight, Loader, LoadingManager, Mesh, MeshStandardMaterial, NeutralToneMapping,
  PerspectiveCamera, PMREMGenerator, Quaternion, Scene, Texture, Vector3, WebGLRenderer, type BufferGeometry,
  type Material, type Object3D,
} from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { STLLoader } from "three/addons/loaders/STLLoader.js";
import { ThreeMFLoader } from "three/addons/loaders/3MFLoader.js";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";
import type { ModelFormat } from "./models";
import { sym } from "./icons";

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

/** Read a glTF; the companion files it names come from `companions`, and
 *  nothing else is fetched. */
async function gltfScene(bytes: ArrayBuffer, companions: Companions): Promise<Object3D> {
  const urls = new Map([...companions].map(([uri, data]) => [uri, URL.createObjectURL(new Blob([data]))]));
  const manager = new LoadingManager();
  // An embedded data URI loads as written; any other file not read for it is empty, so it fails here.
  manager.setURLModifier((url) => urls.get(url) ?? (url.startsWith("data:") ? url : "data:application/octet-stream;base64,"));
  try {
    const gltf = await new GLTFLoader(manager).parseAsync(bytes, "");
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

/** Centre the model at the origin, inside a pivot, and say how big it is. */
function centred(object: Object3D): { pivot: Group; size: number } {
  const box = new Box3().setFromObject(object);
  object.position.sub(box.getCenter(new Vector3()));
  const pivot = new Group();
  pivot.add(object);
  return { pivot, size: box.getSize(new Vector3()).length() || 1 };
}

/** Look at the whole model from the front, a little above and to the right. */
function frame(camera: PerspectiveCamera, controls: OrbitControls, size: number): void {
  const distance = (size / 2 / Math.tan((camera.fov * Math.PI) / 360)) * 1.15;
  camera.position.set(0.55, 0.45, 1).normalize().multiplyScalar(distance);
  camera.near = size / 100;
  camera.far = size * 100;
  camera.updateProjectionMatrix();
  controls.target.set(0, 0, 0);
  controls.update();
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
  canvas.setAttribute("aria-label", "3D model: drag or use the arrow keys to turn, scroll to zoom");
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
  const camera = new PerspectiveCamera(35, 1, 0.1, 1000);
  // Up and to the left of the eye, so the faces you see are shaded apart.
  const key = new DirectionalLight(0xffffff, 1.6);
  key.position.set(-1, 1.5, 0.5);
  camera.add(key);
  scene.add(camera);

  const controls = new OrbitControls(camera, canvas);
  const render = () => renderer.render(scene, camera);
  controls.addEventListener("change", render);
  frame(camera, controls, size);
  controls.saveState();

  const turns = turner(pivot, render);
  const turn = (t: Turn) => {
    // Pitch tips the model about the view's own horizontal axis.
    turns.turn(t, new Vector3().setFromMatrixColumn(camera.matrixWorld, 0));
    if (t === "reset") controls.reset();
  };
  wrap.appendChild(modelControls(turn));
  canvas.addEventListener("keydown", (event) => {
    const t = turnForKey(event.key);
    if (!t) return;
    event.preventDefault();
    event.stopPropagation();
    turn(t);
  });

  const resize = () => {
    const width = canvas.clientWidth || 1;
    const height = canvas.clientHeight || 1;
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
    render();
  };
  const observer = new ResizeObserver(resize);
  observer.observe(canvas);
  resize();

  return {
    dispose() {
      turns.stop();
      observer.disconnect();
      controls.dispose();
      disposeScene(scene);
      scene.environment?.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
      wrap.remove();
    },
  };
}

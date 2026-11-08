// The Space preview's 3D viewer, built on three.js and loaded only when a
// model is previewed, so the app starts without it. `loadModel` turns a
// file's bytes into a three.js object (no WebGL needed, so it is tested);
// `showModel` draws it on a canvas you can turn and zoom, and its `dispose`
// gives the GPU memory back when the preview closes.

import {
  Box3, DirectionalLight, Group, HemisphereLight, Loader, LoadingManager, Mesh, MeshStandardMaterial, NeutralToneMapping,
  PerspectiveCamera, PMREMGenerator, Scene, Texture, Vector3, WebGLRenderer, type BufferGeometry, type Material,
  type Object3D,
} from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { STLLoader } from "three/addons/loaders/STLLoader.js";
import { ThreeMFLoader } from "three/addons/loaders/3MFLoader.js";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";
import type { ModelFormat } from "./models";

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

/**
 * Draw `object` in `host`, turned with the pointer and zoomed with the wheel.
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
  canvas.setAttribute("aria-label", "3D model: drag to turn, scroll to zoom");
  host.appendChild(canvas);

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
      observer.disconnect();
      controls.dispose();
      disposeScene(scene);
      scene.environment?.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
      canvas.remove();
    },
  };
}

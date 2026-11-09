// placeScene: an Arbesk scene's parts, each placed once per placement, as
// copies in one group (src/model-viewer.ts).
import { expect, test } from "vitest";
import {
  Bone, Box3, BoxGeometry, BufferAttribute, BufferGeometry, Group, Mesh, MeshStandardMaterial, SkinnedMesh, Skeleton, Vector3,
  type Object3D,
} from "three";
import { loadModel, loadScenePart, placeScene, type ScenePlacement } from "../src/model-viewer";
import { sample3mf, sampleGlb } from "../dev/mock-tauri/models";

const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
const translation = (x: number, y: number, z: number) => [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, x, y, z, 1];

/** A box model: a group holding a "seat" mesh and a "leg" mesh, grey. */
function box(): Object3D {
  const model = new Group();
  for (const name of ["seat", "leg"]) {
    const mesh = new Mesh(new BoxGeometry(1, 1, 1), new MeshStandardMaterial({ color: "#808080" }));
    mesh.name = name;
    model.add(mesh);
  }
  return model;
}

const place = (over: Partial<ScenePlacement> = {}): ScenePlacement => ({ matrix: IDENTITY, color: null, mesh_colors: {}, scale: [1, 1, 1], ...over });

function colourOf(copy: Object3D, name: string): string {
  const mesh = copy.getObjectByName(name) as Mesh;
  return `#${(mesh.material as MeshStandardMaterial).color.getHexString()}`;
}

test("each placement is a copy at its matrix", () => {
  const group = placeScene([{ object: box(), placements: [place(), place({ matrix: translation(3, 0, 0) })] }]);
  expect(group.children.map((copy) => copy.position.x)).toEqual([0, 3]);
});

test("a tinted copy changes colour and the others keep theirs", () => {
  const group = placeScene([{ object: box(), placements: [place(), place({ color: "#ff0000" })] }]);
  const [plain, tinted] = group.children;
  expect(colourOf(plain, "seat")).toBe("#808080");
  expect(colourOf(tinted, "seat")).toBe("#ff0000");
  expect(colourOf(tinted, "leg")).toBe("#ff0000");
});

test("a mesh's own colour beats the copy's", () => {
  const group = placeScene([{ object: box(), placements: [place({ color: "#ff0000", mesh_colors: { seat: "#00ff00" } })] }]);
  expect(colourOf(group.children[0], "seat")).toBe("#00ff00");
  expect(colourOf(group.children[0], "leg")).toBe("#ff0000");
});

const bounds = (object: Object3D) => {
  object.updateMatrixWorld(true);
  return new Box3().setFromObject(object);
};
const size = (object: Object3D) => bounds(object).getSize(new Vector3());
const centre = (object: Object3D) => bounds(object).getCenter(new Vector3());
const near = (a: Vector3, b: Vector3) => a.distanceTo(b) < 1e-6;

test("a copy's scale multiplies its matrix's", () => {
  const scaled = [2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1];
  const group = placeScene([{ object: box(), placements: [place({ matrix: scaled, scale: [2, 1, 1] })] }]);
  expect(near(size(group.children[0]), new Vector3(4, 2, 2))).toBe(true);
});

// As the Studio's centerImportedAsset: the middle of a model's box sits on its place.
test("each copy's model is centred on its place", () => {
  const model = box();
  model.children.forEach((mesh) => mesh.position.set(5, 1, -2));
  const group = placeScene([{ object: model, placements: [place({ matrix: translation(3, 0, 0) }), place()] }]);
  expect(near(centre(group.children[0]), new Vector3(3, 0, 0))).toBe(true);
  expect(near(centre(group.children[1]), new Vector3(0, 0, 0))).toBe(true);
});

const buffer = (bytes: Uint8Array) => bytes.slice().buffer;

// The Studio draws a 3MF in metres, from the unit it declares; a GLB is in metres already.
test("a 3MF scene part is in metres", async () => {
  const raw = size(await loadModel("3mf", buffer(sample3mf())));
  for (const [unit, metres] of [["millimeter", 0.001], ["inch", 0.0254], ["meter", 1], ["furlong", 0.001]] as const) {
    const part = size(await loadScenePart(buffer(sample3mf(unit))));
    expect(near(part, raw.clone().multiplyScalar(metres)), unit).toBe(true);
  }
  const glb = buffer(sampleGlb());
  expect(near(size(await loadScenePart(glb)), size(await loadModel("glb", glb)))).toBe(true);
});

test("a skinned part's bones come with its placement", () => {
  const bone = new Bone();
  bone.position.set(1, 0, 0);
  const geometry = new BufferGeometry();
  geometry.setAttribute("position", new BufferAttribute(new Float32Array([0, 0, 0]), 3));
  geometry.setAttribute("skinIndex", new BufferAttribute(new Uint16Array([0, 0, 0, 0]), 4));
  geometry.setAttribute("skinWeight", new BufferAttribute(new Float32Array([1, 0, 0, 0]), 4));
  const mesh = new SkinnedMesh(geometry, new MeshStandardMaterial());
  mesh.name = "skinned";
  mesh.add(bone);
  mesh.bind(new Skeleton([bone]));
  const copy = placeScene([{ object: new Group().add(mesh), placements: [place()] }]);
  const skinned = copy.getObjectByName("skinned") as SkinnedMesh;
  const clonedBone = skinned.skeleton.bones[0];
  // A plain clone would share the original's bone, so turning the copy would
  // leave the skinned mesh where it was.
  expect(clonedBone).not.toBe(bone);
  const world = () => clonedBone.getWorldPosition(new Vector3());
  const before = world();
  copy.children[0].rotation.y = Math.PI / 2;
  copy.updateMatrixWorld(true);
  expect(before.distanceTo(world())).toBeGreaterThan(0.5);
  expect(bone.getWorldPosition(new Vector3()).distanceTo(before)).toBe(0);
});

test("parts keep their own transforms inside each copy", () => {
  const model = box();
  model.rotation.x = -Math.PI / 2; // as a 3MF stood up from Z-up
  model.name = "copy-of-model";
  const group = placeScene([{ object: model, placements: [place({ matrix: translation(1, 0, 0) })] }]);
  const copy = group.children[0].getObjectByName("copy-of-model")!;
  expect(copy.rotation.x).toBeCloseTo(-Math.PI / 2);
});

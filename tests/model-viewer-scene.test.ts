// placeScene: an Arbesk scene's parts, each placed once per placement, as
// copies in one group (src/model-viewer.ts).
import { expect, test } from "vitest";
import { BoxGeometry, Group, Mesh, MeshStandardMaterial, Vector3, type Object3D } from "three";
import { placeScene, type ScenePlacement } from "../src/model-viewer";

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

test("a copy's scale multiplies its matrix's", () => {
  const scaled = [2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1];
  const group = placeScene([{ object: box(), placements: [place({ matrix: scaled, scale: [2, 1, 1] })] }]);
  expect(group.children[0].scale).toEqual(new Vector3(4, 2, 2));
});

test("parts keep their own transforms inside each copy", () => {
  const model = box();
  model.rotation.x = -Math.PI / 2; // as a 3MF stood up from Z-up
  const group = placeScene([{ object: model, placements: [place({ matrix: translation(1, 0, 0) })] }]);
  expect(group.children[0].children[0].rotation.x).toBeCloseTo(-Math.PI / 2);
});

// 3D models in the Space preview, without three.js or the DOM: which files are
// models, and which companion files a .gltf may load. The viewer itself
// (src/model-viewer.ts) is loaded only when a model is previewed.

export type ModelFormat = "gltf" | "glb" | "stl" | "3mf";

const FORMATS: Record<string, ModelFormat> = { gltf: "gltf", glb: "glb", stl: "stl", "3mf": "3mf" };

/** The model format a file name says, or null for anything else. */
export function modelFormat(name: string): ModelFormat | null {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? (FORMATS[name.slice(dot + 1).toLowerCase()] ?? null) : null;
}

/**
 * A glTF `uri` as a path below the model's folder, or null when it is not
 * one: an embedded data URI, another scheme, an absolute path, a backslash or
 * a `..` that would climb out. Its parts are percent-decoded.
 */
export function companionPath(uri: string): string | null {
  if (/^[a-z][a-z0-9+.-]*:/i.test(uri) || uri.startsWith("/") || uri.includes("\\")) return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(uri);
  } catch {
    return null;
  }
  const parts = decoded.split("/");
  const climbs = parts.some((part) => part === ".." || part === "");
  return climbs || decoded.includes("\\") ? null : parts.filter((part) => part !== ".").join("/");
}

type GltfJson = { buffers?: Array<{ uri?: unknown }>; images?: Array<{ uri?: unknown }> };

/** The companion files a .gltf names (its buffers' and images' URIs), each once, as written. */
export function gltfCompanions(json: unknown): string[] {
  const doc = (json ?? {}) as GltfJson;
  const uris = [...(doc.buffers ?? []), ...(doc.images ?? [])]
    .map((item) => item?.uri)
    .filter((uri): uri is string => typeof uri === "string" && companionPath(uri) !== null);
  return [...new Set(uris)];
}

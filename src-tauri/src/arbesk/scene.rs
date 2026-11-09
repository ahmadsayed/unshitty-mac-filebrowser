//! An Arbesk asset as a scene: its manifest's nodes, each a stored model or
//! a link to another asset, placed by its `transform_matrix`. Resolving one
//! gives the distinct models (parts) and every place each goes, in
//! right-handed coordinates, with the node's colour and scale. The Studio's
//! own loader (`scene-loader.ts`) is the reference.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;

pub use crate::arbesk::compose::Format;

/// A 4×4 matrix, column-major, as the manifest's `transform_matrix`.
pub type Matrix = [f64; 16];

pub const IDENTITY: Matrix = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// How many links deep a scene is followed, as the Studio's `MAX_CHILD_ASSET_DEPTH`.
pub const MAX_LINK_DEPTH: usize = 5;

/// At most this many copies in one scene: links multiply, and a scene of
/// links to links could ask for millions.
pub const MAX_PLACEMENTS: usize = 2_000;

/// Until 2026-10-17 06:19:37 UTC the Studio drew a 3MF at one unit per
/// millimetre, so an older manifest places its 3MF parts in millimetres
/// (the Studio's `normalizeLegacy3mfPlacements`); now it draws them in metres.
pub const LEGACY_3MF_CUTOFF_MS: i64 = 1_792_217_977_000;

/// One place a part goes.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Placement {
    /// Column-major, right-handed.
    pub matrix: Vec<f64>,
    /// `#rrggbb` for every mesh, unless `mesh_colors` names it.
    pub color: Option<String>,
    pub mesh_colors: BTreeMap<String, String>,
    pub scale: [f64; 3],
}

/// One stored model and every place it goes.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Part {
    /// The source's CID.
    pub key: String,
    /// What the preview gets: "glb" or "3mf".
    pub format: &'static str,
    pub placements: Vec<Placement>,
    /// Stored as the model file itself (older saves), not as a source to compose.
    #[serde(skip)]
    pub raw: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
pub struct Scene {
    pub parts: Vec<Part>,
}

/// The scenes the preview resolved lately, by path, so each of its parts
/// is checked against the scene without resolving it again.
#[derive(Default)]
pub struct Scenes(Mutex<HashMap<String, Arc<Scene>>>);

/// How many scenes are kept: the previews of the last few assets.
const KEPT_SCENES: usize = 8;

impl Scenes {
    pub fn keep(&self, path: &str, scene: Scene) -> Arc<Scene> {
        let mut kept = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if kept.len() >= KEPT_SCENES && !kept.contains_key(path) {
            kept.clear();
        }
        let scene = Arc::new(scene);
        kept.insert(path.to_string(), scene.clone());
        scene
    }

    fn get(&self, path: &str) -> Option<Arc<Scene>> {
        let kept = self.0.lock().unwrap_or_else(|e| e.into_inner());
        kept.get(path).cloned()
    }

    /// The scene kept for `path`, else the one `make` resolves, kept.
    pub fn get_or_keep(
        &self,
        path: &str,
        make: impl FnOnce() -> Result<Scene, String>,
    ) -> Result<Arc<Scene>, String> {
        match self.get(path) {
            Some(scene) => Ok(scene),
            None => Ok(self.keep(path, make()?)),
        }
    }
}

/// What resolving needs from Arbesk.
pub trait Links {
    /// The JSON document at `cid`.
    fn manifest(&self, cid: &str) -> Result<Value, String>;
    /// The manifest CID of the asset a `child_ref` names.
    fn linked(&self, child_ref: &Value) -> Result<String, String>;
    /// What a stored source is.
    fn format(&self, source_cid: &str) -> Result<Format, String>;
}

/// `a · b`, both column-major.
pub fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
        }
    }
    out
}

/// `X · m · X` with `X = diag(−1, 1, 1, 1)`: the Studio places models in
/// Babylon's left-handed space under a root that mirrors X; three.js and
/// glTF are right-handed.
pub fn to_right_handed(m: &Matrix) -> Matrix {
    let mut out = *m;
    for col in 0..4 {
        for row in 0..4 {
            if (row == 0) != (col == 0) {
                out[col * 4 + row] = -m[col * 4 + row];
            }
        }
    }
    out
}

/// The parts of the asset whose manifest is at `asset_cid`, in first-seen
/// order. Whatever can't be resolved is left out.
pub fn resolve(asset_cid: &str, links: &dyn Links) -> Scene {
    let mut walk = Walk {
        links,
        scene: Scene::default(),
        seen: HashSet::new(),
        linked: HashMap::new(),
        placed: 0,
    };
    walk.asset(asset_cid, &IDENTITY, 0);
    walk.scene
}

struct Walk<'a> {
    links: &'a dyn Links,
    scene: Scene,
    /// Manifest CIDs on the way down: one met again is a loop.
    seen: HashSet<String>,
    /// Each link's manifest CID, by the link: one asked once.
    linked: HashMap<String, Option<String>>,
    placed: usize,
}

impl Walk<'_> {
    fn asset(&mut self, cid: &str, parent: &Matrix, depth: usize) {
        let Ok(manifest) = self.links.manifest(cid) else {
            return;
        };
        // A manifest that is itself the stored model (older assets).
        if ["buffers", "meshes", "arbesk_format"]
            .iter()
            .any(|key| manifest.get(key).is_some())
        {
            self.place(cid, parent, &Value::Null, None);
            return;
        }
        self.seen.insert(cid.to_string());
        let millimetres = manifest["timestamp"]
            .as_i64()
            .is_some_and(|t| t < LEGACY_3MF_CUTOFF_MS);
        let nodes = manifest
            .pointer("/scene/nodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for node in &nodes {
            self.node(node, parent, depth, millimetres);
        }
        self.seen.remove(cid);
    }

    fn node(&mut self, node: &Value, parent: &Matrix, depth: usize, millimetres: bool) {
        let mut own = node_matrix(node);
        if millimetres && node["source"]["format"] == "3mf" && node.get("child_ref").is_none() {
            own[12..15].iter_mut().for_each(|t| *t *= 0.001);
        }
        let here = multiply(parent, &own);
        if let Some(child_ref) = node.get("child_ref") {
            self.link(child_ref, &here, depth);
        } else if let Some(source) = node.pointer("/source/cid").and_then(Value::as_str) {
            self.place(
                source,
                &here,
                node.get("post_processor").unwrap_or(&Value::Null),
                raw_format(&node["source"]),
            );
        }
    }

    fn link(&mut self, child_ref: &Value, here: &Matrix, depth: usize) {
        if depth >= MAX_LINK_DEPTH {
            return;
        }
        let links = self.links;
        let asked = self
            .linked
            .entry(child_ref.to_string())
            .or_insert_with(|| links.linked(child_ref).ok());
        let Some(cid) = asked.clone() else {
            return;
        };
        if !self.seen.contains(&cid) {
            self.asset(&cid, here, depth + 1);
        }
    }

    /// One more place for the source at `key`.
    /// `raw` is the preview format of a source stored as the model file itself.
    fn place(&mut self, key: &str, matrix: &Matrix, post: &Value, raw: Option<&'static str>) {
        if self.placed >= MAX_PLACEMENTS {
            return;
        }
        let index = match self.scene.parts.iter().position(|part| part.key == key) {
            Some(index) => index,
            None => {
                let format = match raw {
                    Some(format) => format,
                    None => match self.links.format(key) {
                        Ok(format) => format.preview_ext(),
                        Err(_) => return,
                    },
                };
                self.scene.parts.push(Part {
                    key: key.to_string(),
                    format,
                    placements: Vec::new(),
                    raw: raw.is_some(),
                });
                self.scene.parts.len() - 1
            }
        };
        let placement = Placement {
            matrix: to_right_handed(matrix).to_vec(),
            color: post.get("color").and_then(Value::as_str).map(String::from),
            mesh_colors: mesh_colors(post),
            scale: ["x", "y", "z"].map(|axis| {
                post.pointer(&format!("/scale/{axis}"))
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0)
            }),
        };
        self.placed += 1;
        self.scene.parts[index].placements.push(placement);
    }
}

/// The preview format of a source that is the model file itself, as older
/// saves store it (the Studio's raw GLB and raw 3MF loads): a `glb`, or a
/// `3mf` whose path isn't the stored form's `composite.3mf.json`.
fn raw_format(source: &Value) -> Option<&'static str> {
    let path = source["path"].as_str().unwrap_or_default();
    match source["format"].as_str() {
        Some("glb") => Some("glb"),
        Some("3mf") if !path.ends_with(".json") => Some("3mf"),
        _ => None,
    }
}

/// A node's `transform_matrix`, or the identity without a valid one.
fn node_matrix(node: &Value) -> Matrix {
    let numbers: Option<Vec<f64>> = node
        .get("transform_matrix")
        .and_then(Value::as_array)
        .map(|m| m.iter().filter_map(Value::as_f64).collect());
    numbers.and_then(|m| m.try_into().ok()).unwrap_or(IDENTITY)
}

/// `meshOverrides[name].color`, by mesh name.
fn mesh_colors(post: &Value) -> BTreeMap<String, String> {
    let overrides = post.get("meshOverrides").and_then(Value::as_object);
    overrides
        .into_iter()
        .flatten()
        .filter_map(|(name, o)| Some((name.clone(), o.get("color")?.as_str()?.to_string())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const fn translation(x: f64, y: f64, z: f64) -> Matrix {
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, z, 1.0,
        ]
    }

    /// A turn of +90° about Y, column-major.
    const TURN_Y_90: Matrix = [
        0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    const TURN_Y_MINUS_90: Matrix = [
        0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    #[derive(Default)]
    struct Fake {
        manifests: HashMap<String, Value>,
        links: HashMap<String, String>,
        formats: HashMap<String, Format>,
        /// How many times `linked` and `format` were asked.
        asked: std::cell::Cell<usize>,
        formats_asked: std::cell::Cell<usize>,
    }

    impl Fake {
        fn asset(&mut self, cid: &str, nodes: Value) {
            self.manifests
                .insert(cid.into(), json!({ "scene": { "nodes": nodes } }));
        }
        /// `assetID` → the asset's manifest CID.
        fn link(&mut self, asset_id: &str, cid: &str) {
            self.links.insert(asset_id.into(), cid.into());
        }
        fn source(&mut self, cid: &str, format: Format) {
            self.formats.insert(cid.into(), format);
        }
    }

    impl Links for Fake {
        fn manifest(&self, cid: &str) -> Result<Value, String> {
            self.manifests
                .get(cid)
                .cloned()
                .ok_or_else(|| format!("no manifest {cid}"))
        }
        fn linked(&self, child_ref: &Value) -> Result<String, String> {
            self.asked.set(self.asked.get() + 1);
            let id = child_ref["assetID"].as_str().unwrap_or_default();
            self.links
                .get(id)
                .cloned()
                .ok_or_else(|| format!("no link {id}"))
        }
        fn format(&self, source_cid: &str) -> Result<Format, String> {
            self.formats_asked.set(self.formats_asked.get() + 1);
            self.formats
                .get(source_cid)
                .copied()
                .ok_or_else(|| format!("no source {source_cid}"))
        }
    }

    fn source_node(cid: &str, matrix: Option<Matrix>) -> Value {
        let mut node = json!({ "source": { "cid": cid, "format": "gltf" } });
        if let Some(m) = matrix {
            node["transform_matrix"] = json!(m.to_vec());
        }
        node
    }

    fn link_node(asset_id: &str, matrix: Matrix) -> Value {
        json!({ "transform_matrix": m(matrix), "child_ref": { "collection": { "chainId": 84532, "tokenId": "1" }, "assetID": asset_id } })
    }

    fn m(matrix: Matrix) -> Vec<f64> {
        matrix.to_vec()
    }

    fn close(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn matrices_multiply_column_major() {
        let product = multiply(&translation(1.0, 0.0, 0.0), &TURN_Y_90);
        assert!(close(
            &product,
            &[
                0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0
            ]
        ));
        assert!(close(&multiply(&IDENTITY, &TURN_Y_90), &TURN_Y_90));
    }

    #[test]
    fn the_studios_left_handed_matrices_become_right_handed() {
        assert!(close(
            &to_right_handed(&translation(2.0, 1.0, 3.0)),
            &translation(-2.0, 1.0, 3.0)
        ));
        assert!(close(&to_right_handed(&TURN_Y_90), &TURN_Y_MINUS_90));
        assert!(close(&to_right_handed(&IDENTITY), &IDENTITY));
    }

    #[test]
    fn a_single_model_is_one_part_placed_as_is() {
        let mut fake = Fake::default();
        fake.asset("bafyA", json!([source_node("bafyS", None)]));
        fake.source("bafyS", Format::Gltf);
        let scene = resolve("bafyA", &fake);
        assert_eq!(scene.parts.len(), 1);
        let part = &scene.parts[0];
        assert_eq!((part.key.as_str(), part.format), ("bafyS", "glb"));
        assert_eq!(
            part.placements,
            [Placement {
                matrix: m(IDENTITY),
                color: None,
                mesh_colors: Default::default(),
                scale: [1.0, 1.0, 1.0]
            }]
        );
    }

    #[test]
    fn a_manifest_that_is_itself_the_model_is_one_part() {
        let mut fake = Fake::default();
        fake.manifests
            .insert("bafyA".into(), json!({ "buffers": [], "meshes": [] }));
        fake.source("bafyA", Format::Gltf);
        let scene = resolve("bafyA", &fake);
        assert_eq!(scene.parts.len(), 1);
        assert_eq!(scene.parts[0].key, "bafyA");
    }

    #[test]
    fn the_table_scene() {
        let table = [
            4.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 1.27, 0.0, 1.0,
        ];
        let left = translation(-2.57, 1.95, 0.0);
        let right = translation(2.08, 1.95, 0.03);
        let mut fake = Fake::default();
        fake.asset(
            "bafyTable",
            json!([
                source_node("bafyT", Some(table)),
                link_node("chair", left),
                link_node("chair", right)
            ]),
        );
        fake.asset("bafyChair", json!([source_node("bafyS", None)]));
        fake.link("chair", "bafyChair");
        fake.source("bafyT", Format::Gltf);
        fake.source("bafyS", Format::Gltf);
        let scene = resolve("bafyTable", &fake);
        let keys: Vec<&str> = scene.parts.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["bafyT", "bafyS"]);
        assert!(close(
            &scene.parts[0].placements[0].matrix,
            &to_right_handed(&table)
        ));
        let chairs: Vec<&Vec<f64>> = scene.parts[1]
            .placements
            .iter()
            .map(|p| &p.matrix)
            .collect();
        assert_eq!(chairs.len(), 2);
        assert!(close(chairs[0], &to_right_handed(&left)));
        assert!(close(chairs[1], &to_right_handed(&right)));
    }

    #[test]
    fn nested_links_multiply() {
        let mut fake = Fake::default();
        fake.asset("bafyA", json!([link_node("b", translation(1.0, 0.0, 0.0))]));
        fake.asset("bafyB", json!([link_node("c", translation(0.0, 2.0, 0.0))]));
        fake.asset("bafyC", json!([source_node("bafyS", None)]));
        fake.link("b", "bafyB");
        fake.link("c", "bafyC");
        fake.source("bafyS", Format::ThreeMf);
        let scene = resolve("bafyA", &fake);
        assert_eq!(scene.parts[0].format, "3mf");
        assert!(close(
            &scene.parts[0].placements[0].matrix,
            &translation(-1.0, 2.0, 0.0)
        ));
    }

    #[test]
    fn a_cycle_is_left_out() {
        let mut fake = Fake::default();
        fake.asset("bafyA", json!([link_node("b", IDENTITY)]));
        fake.asset(
            "bafyB",
            json!([link_node("a", IDENTITY), source_node("bafyS", None)]),
        );
        fake.link("a", "bafyA");
        fake.link("b", "bafyB");
        fake.source("bafyS", Format::Gltf);
        let scene = resolve("bafyA", &fake);
        assert_eq!(scene.parts.len(), 1);
        assert_eq!(scene.parts[0].placements.len(), 1);
    }

    #[test]
    fn past_five_levels_is_left_out() {
        let mut fake = Fake::default();
        for level in 0..7 {
            fake.asset(
                &format!("bafyL{level}"),
                json!([link_node(&format!("l{}", level + 1), IDENTITY)]),
            );
            fake.link(&format!("l{}", level + 1), &format!("bafyL{}", level + 1));
        }
        fake.asset("bafyL7", json!([source_node("bafyS", None)]));
        fake.source("bafyS", Format::Gltf);
        assert!(resolve("bafyL0", &fake).parts.is_empty());
        // Five links down is still within reach.
        let mut near = Fake::default();
        for level in 0..5 {
            near.asset(
                &format!("bafyL{level}"),
                json!([link_node(&format!("l{}", level + 1), IDENTITY)]),
            );
            near.link(&format!("l{}", level + 1), &format!("bafyL{}", level + 1));
        }
        near.asset("bafyL5", json!([source_node("bafyS", None)]));
        near.source("bafyS", Format::Gltf);
        assert_eq!(resolve("bafyL0", &near).parts.len(), 1);
    }

    #[test]
    fn an_unresolvable_link_or_unknown_source_is_left_out() {
        let mut fake = Fake::default();
        fake.asset(
            "bafyA",
            json!([
                link_node("missing", IDENTITY),
                source_node("bafyOdd", None),
                source_node("bafyS", None)
            ]),
        );
        fake.source("bafyS", Format::Gltf);
        let scene = resolve("bafyA", &fake);
        assert_eq!(
            scene
                .parts
                .iter()
                .map(|p| p.key.as_str())
                .collect::<Vec<_>>(),
            ["bafyS"]
        );
        assert!(resolve("bafyNowhere", &fake).parts.is_empty());
    }

    #[test]
    fn colour_mesh_colours_and_scale_are_carried() {
        let mut fake = Fake::default();
        let mut node = source_node("bafyS", None);
        node["post_processor"] = json!({ "color": "#ff0000", "meshOverrides": { "seat": { "color": "#00ff00" } }, "scale": { "x": 2 } });
        fake.asset("bafyA", json!([node]));
        fake.source("bafyS", Format::Gltf);
        let placement = &resolve("bafyA", &fake).parts[0].placements[0];
        assert_eq!(placement.color.as_deref(), Some("#ff0000"));
        assert_eq!(
            placement.mesh_colors.get("seat").map(String::as_str),
            Some("#00ff00")
        );
        assert_eq!(placement.scale, [2.0, 1.0, 1.0]);
    }

    #[test]
    fn a_scene_starting_with_a_link() {
        let mut fake = Fake::default();
        fake.asset("bafyA", json!([link_node("b", translation(0.0, 0.0, 5.0))]));
        fake.asset("bafyB", json!([source_node("bafyS", None)]));
        fake.link("b", "bafyB");
        fake.source("bafyS", Format::Gltf);
        let scene = resolve("bafyA", &fake);
        assert!(close(
            &scene.parts[0].placements[0].matrix,
            &translation(0.0, 0.0, 5.0)
        ));
    }

    #[test]
    fn a_link_is_asked_once_however_often_it_is_placed() {
        let mut fake = Fake::default();
        let chairs: Vec<Value> = (0..4)
            .map(|i| link_node("chair", translation(f64::from(i), 0.0, 0.0)))
            .collect();
        fake.asset("bafyA", json!(chairs));
        fake.asset("bafyChair", json!([source_node("bafyS", None)]));
        fake.link("chair", "bafyChair");
        fake.source("bafyS", Format::Gltf);
        assert_eq!(resolve("bafyA", &fake).parts[0].placements.len(), 4);
        assert_eq!((fake.asked.get(), fake.formats_asked.get()), (1, 1));
    }

    #[test]
    fn placements_stop_at_the_cap() {
        let mut fake = Fake::default();
        fake.asset("bafyA", json!(vec![link_node("row", IDENTITY); 60]));
        fake.asset("bafyRow", json!(vec![source_node("bafyS", None); 60]));
        fake.link("row", "bafyRow");
        fake.source("bafyS", Format::Gltf);
        let scene = resolve("bafyA", &fake);
        assert_eq!(scene.parts[0].placements.len(), MAX_PLACEMENTS);
    }

    fn three_mf_node(y: f64) -> Value {
        json!({ "source": { "cid": "bafy3", "format": "3mf" }, "transform_matrix": m(translation(0.0, y, 0.0)) })
    }

    #[test]
    fn a_3mf_placed_before_metres_moves_in_millimetres() {
        let mut fake = Fake::default();
        let nodes = json!([
            three_mf_node(500.0),
            source_node("bafyS", Some(translation(0.0, 2.0, 0.0)))
        ]);
        fake.manifests.insert(
            "bafyOld".into(),
            json!({ "timestamp": LEGACY_3MF_CUTOFF_MS - 1, "scene": { "nodes": nodes } }),
        );
        fake.manifests.insert(
            "bafyNew".into(),
            json!({ "timestamp": LEGACY_3MF_CUTOFF_MS, "scene": { "nodes": nodes } }),
        );
        fake.manifests
            .insert("bafyUndated".into(), json!({ "scene": { "nodes": nodes } }));
        fake.source("bafy3", Format::ThreeMf);
        fake.source("bafyS", Format::Gltf);
        let lift = |cid: &str| {
            resolve(cid, &fake)
                .parts
                .iter()
                .map(|p| p.placements[0].matrix[13])
                .collect::<Vec<_>>()
        };
        assert_eq!(lift("bafyOld"), [0.5, 2.0]);
        assert_eq!(lift("bafyNew"), [500.0, 2.0]);
        assert_eq!(lift("bafyUndated"), [500.0, 2.0]);
    }

    #[test]
    fn the_last_scenes_are_kept() {
        let scenes = Scenes::default();
        let one = |key: &str| Scene {
            parts: vec![Part {
                key: key.into(),
                format: "glb",
                placements: vec![],
                raw: false,
            }],
        };
        scenes.keep("a", one("x"));
        assert_eq!(scenes.get("a").unwrap().parts[0].key, "x");
        scenes.keep("a", one("y"));
        assert_eq!(scenes.get("a").unwrap().parts[0].key, "y");
        for i in 0..KEPT_SCENES {
            scenes.keep(&format!("p{i}"), one("z"));
        }
        assert!(scenes.get("a").is_none());
        assert!(scenes.get(&format!("p{}", KEPT_SCENES - 1)).is_some());
        assert!(scenes.get("nowhere").is_none());
        let last = format!("p{}", KEPT_SCENES - 1);
        let made = scenes.get_or_keep(&last, || Err("not asked".into()));
        assert_eq!(made.unwrap().parts[0].key, "z");
        assert!(scenes.get_or_keep("new", || Err("refused".into())).is_err());
        let made = scenes.get_or_keep("new", || Ok(one("n"))).unwrap();
        assert_eq!(
            (made.parts[0].key.as_str(), scenes.get("new").is_some()),
            ("n", true)
        );
    }

    #[test]
    fn an_older_saves_raw_file_is_a_part_by_its_format() {
        // Older saves name the model file itself: a GLB, or a zipped 3MF.
        let mut fake = Fake::default();
        let raw = |cid: &str, path: &str, format: &str| json!({ "source": { "cid": cid, "path": path, "format": format } });
        fake.asset(
            "bafyA",
            json!([
                raw("bafyGlb", "asset.glb", "glb"),
                raw("bafy3mf", "asset.3mf", "3mf")
            ]),
        );
        let scene = resolve("bafyA", &fake);
        let parts: Vec<(&str, &str, bool)> = scene
            .parts
            .iter()
            .map(|p| (p.key.as_str(), p.format, p.raw))
            .collect();
        assert_eq!(parts, [("bafyGlb", "glb", true), ("bafy3mf", "3mf", true)]);
        // Known from the node alone: nothing is fetched to tell.
        assert_eq!(fake.formats_asked.get(), 0);
        // A stored (composite) 3MF is not raw.
        let mut stored = Fake::default();
        stored.asset("bafyB", json!([raw("bafyC3", "composite.3mf.json", "3mf")]));
        stored.source("bafyC3", Format::ThreeMf);
        assert!(!resolve("bafyB", &stored).parts[0].raw);
    }
}

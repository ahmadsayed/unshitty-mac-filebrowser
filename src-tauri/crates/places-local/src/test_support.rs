//! Helpers shared by tests in several modules.

use std::path::{Path, PathBuf};

/// Where the source guards look, as the app's `test_support` sees them: the
/// app's `src`, and the `src` of every crate under `crates/`, each with the
/// prefix its files are named by in the guards' allowed lists (`""`,
/// `"crates/<name>/src/"`). This crate lives in `src-tauri/crates/`, so the
/// app is two folders up.
pub fn source_roots() -> Vec<(PathBuf, String)> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut crates: Vec<String> = std::fs::read_dir(manifest.join("crates"))
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().join("src").is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    crates.sort();
    let mut roots = vec![(manifest.join("src"), String::new())];
    for name in crates {
        roots.push((manifest.join("crates").join(&name).join("src"), format!("crates/{name}/src/")));
    }
    roots
}

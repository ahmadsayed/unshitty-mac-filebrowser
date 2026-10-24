//! `versioned.json`: the roots whose histories this app watches. Paths are
//! canonical; writes go through the existing temp-file+rename helper.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config_store;

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default = "one")]
    version: u32,
    #[serde(default)]
    roots: Vec<String>,
}

fn one() -> u32 {
    1
}

pub fn read(path: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<File>(&text).ok())
        .map(|file| file.roots.into_iter().map(PathBuf::from).collect())
        .unwrap_or_default()
}

pub fn write(path: &Path, roots: &[PathBuf]) -> Result<(), String> {
    let file = File {
        version: 1,
        roots: roots
            .iter()
            .map(|root| root.to_string_lossy().to_string())
            .collect(),
    };
    config_store::write_json(path, &file)
}

pub fn register(path: &Path, root: &Path) -> Result<(), String> {
    let mut roots = read(path);
    if !roots.iter().any(|r| r == root) {
        roots.push(root.to_path_buf());
        write(path, &roots)?;
    }
    Ok(())
}

pub fn unregister(path: &Path, root: &Path) -> Result<(), String> {
    let mut roots = read(path);
    roots.retain(|r| r != root);
    write(path, &roots)
}

/// Rewrite every root at or below `from` to the corresponding path below `to`.
/// Uses component boundaries, so `/foo` never matches `/foobar`.
pub fn rewrite_prefix(path: &Path, from: &Path, to: &Path) -> Result<(), String> {
    let mut roots = read(path);
    let mut changed = false;
    for root in &mut roots {
        if let Ok(suffix) = root.strip_prefix(from) {
            let mut next = to.to_path_buf();
            if !suffix.as_os_str().is_empty() {
                next.push(suffix);
            }
            *root = next;
            changed = true;
        }
    }
    if changed {
        write(path, &roots)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trips_and_dedupes() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("versioned.json");
        let a = PathBuf::from("/home/me/Documents");
        let b = PathBuf::from("/home/me/Projects");
        register(&file, &a).unwrap();
        register(&file, &a).unwrap();
        register(&file, &b).unwrap();
        assert_eq!(read(&file), vec![a.clone(), b.clone()]);
        unregister(&file, &a).unwrap();
        assert_eq!(read(&file), vec![b]);
    }

    #[test]
    fn missing_or_corrupt_is_empty() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("versioned.json");
        assert!(read(&file).is_empty());
        std::fs::write(&file, "{ not json").unwrap();
        assert!(read(&file).is_empty());
    }

    #[test]
    fn rewrites_with_component_boundaries() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("versioned.json");
        write(
            &file,
            &[
                PathBuf::from("/foo"),
                PathBuf::from("/foo/child"),
                PathBuf::from("/foobar"),
            ],
        )
        .unwrap();
        rewrite_prefix(&file, Path::new("/foo"), Path::new("/new")).unwrap();
        assert_eq!(
            read(&file),
            vec![
                PathBuf::from("/new"),
                PathBuf::from("/new/child"),
                PathBuf::from("/foobar"),
            ]
        );
    }
}

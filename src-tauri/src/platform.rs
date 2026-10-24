//! Everything that differs between macOS and Linux, in plain functions so it
//! can be unit-tested without Tauri (see AGENTS.md).
//!
//! Keep this file free of `tauri` types; `lib.rs` adapts these to commands.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

pub const IS_MACOS: bool = cfg!(target_os = "macos");

/// Properties label for a folder others can put files into but not look
/// inside (`-wx`). GNOME Files calls it "Write-only"; macOS has a name for
/// exactly this, the Drop Box in every user's Public folder, so say so there.
pub const WRITE_ONLY_FOLDER: &str = if IS_MACOS { "Write-only (Drop Box)" } else { "Write-only" };

/// A mounted volume as the sidebar shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Volume {
    /// Label: the folder name the mount point has.
    pub name: String,
    pub path: String,
}

/// Folders that hold mounted volumes on this platform.
///
/// macOS puts them all in `/Volumes`. Linux has no single place: udisks2
/// mounts removable drives under `/run/media/<user>` on Fedora and
/// `/media/<user>` elsewhere, while system mounts live in `/mnt`.
pub fn volume_roots() -> Vec<PathBuf> {
    if IS_MACOS {
        return vec![PathBuf::from("/Volumes")];
    }
    if cfg!(target_os = "linux") {
        let mut roots = with_containers(&[PathBuf::from("/run/media"), PathBuf::from("/media")]);
        roots.push(PathBuf::from("/mnt"));
        return roots;
    }
    Vec::new()
}

/// `bases` plus every subfolder of them that is not a mount point. Those are
/// the per-user folders udisks2 mounts drives into (`/run/media/<user>`):
/// containers, not volumes. Working this out from the file system instead of
/// `$USER` keeps it right when `$USER` is unset or stale, and keeps other
/// users' folders out of the sidebar (they are usually unreadable, so they
/// add nothing).
fn with_containers(bases: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for base in bases {
        roots.push(base.clone());
        let Ok(entries) = fs::read_dir(base) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if fs::metadata(&path).is_ok_and(|m| m.is_dir()) && !is_mount_point(&path) {
                roots.push(path);
            }
        }
    }
    roots
}

/// Whether `path` is on a different device than its parent folder.
#[cfg(unix)]
fn is_mount_point(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Some(parent) = path.parent() else { return true };
    match (fs::metadata(path), fs::metadata(parent)) {
        (Ok(a), Ok(b)) => a.dev() != b.dev(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_mount_point(_: &Path) -> bool {
    false
}

/// The volumes visible under `roots`: their subfolders, without hidden ones,
/// without files, without folders that are roots themselves (containers like
/// `/run/media/<user>`), and without duplicates (a drive can be reachable from
/// more than one root).
pub fn volumes(roots: &[PathBuf]) -> Vec<Volume> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut found = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            // `/media/<user>` and `/run/media/<user>` are containers for
            // volumes, not volumes themselves; they are also in `roots`.
            if roots.contains(&path) {
                continue;
            }
            if !fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            // Resolve duplicates; a mount point that can't be resolved still
            // counts as itself.
            let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !seen.insert(key) {
                continue;
            }
            found.push(Volume { name, path: path.to_string_lossy().to_string() });
        }
    }
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    found
}

/// Decode `%XX` escapes, as freedesktop `.trashinfo` files and `file://` URIs
/// both use them. Invalid escapes are kept verbatim.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The local path of a `file://` URI, if it is one.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Skip the (empty) authority: file:///home/me/x -> /home/me/x
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(at) => &rest[at..],
        None => return None,
    };
    let decoded = percent_decode(path);
    decoded.starts_with('/').then(|| PathBuf::from(decoded))
}

/// Read a text file, or `None` when it is missing or unreadable.
pub fn read_text(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

/// `$XDG_DATA_HOME`, or its default `~/.local/share`. Used for the Trash and
/// the recently-used list on Linux.
pub fn data_home(home: &Path) -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local/share"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn volumes_lists_only_folders_and_skips_hidden_ones() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("media");
        fs::create_dir_all(root.join("Backup Drive")).unwrap();
        fs::create_dir_all(root.join("USB")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("not-a-volume.txt"), "x").unwrap();

        let volumes = volumes(&[root]);
        let names: Vec<&str> = volumes.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["Backup Drive", "USB"]);
        assert!(volumes[1].path.ends_with("USB"));
    }

    #[cfg(unix)]
    #[test]
    fn volumes_dedupes_across_roots_and_ignores_missing_roots() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(a.join("Disk")).unwrap();
        fs::create_dir_all(&b).unwrap();
        // The same mount reachable from two roots, as /media and /run/media
        // can be.
        std::os::unix::fs::symlink(a.join("Disk"), b.join("Disk")).unwrap();

        let listed = volumes(&[a, b, dir.path().join("nope")]);
        assert_eq!(listed.len(), 1, "the same mount seen twice counts once");

        assert!(volumes(&[dir.path().join("nope")]).is_empty());
    }

    #[test]
    fn user_folders_are_containers_whatever_the_user_is_called() {
        let dir = tempdir().unwrap();
        let media = dir.path().join("run-media");
        // Two users' udisks2 folders; none is a mount point in a temp dir.
        fs::create_dir_all(media.join("ahmed/USB")).unwrap();
        fs::create_dir_all(media.join("someone-else")).unwrap();

        let roots = with_containers(&[media.clone(), dir.path().join("nope")]);
        assert!(roots.contains(&media.join("ahmed")));
        assert!(roots.contains(&media.join("someone-else")));
        let names: Vec<String> = volumes(&roots).into_iter().map(|v| v.name).collect();
        assert_eq!(names, ["USB"]);
    }

    #[cfg(unix)]
    #[test]
    fn the_root_folder_counts_as_a_mount_point() {
        assert!(is_mount_point(Path::new("/")));
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("plain")).unwrap();
        assert!(!is_mount_point(&dir.path().join("plain")));
    }

    #[test]
    fn volume_roots_follow_the_platform() {
        let roots = volume_roots();
        if cfg!(target_os = "macos") {
            assert_eq!(roots, [PathBuf::from("/Volumes")]);
        } else if cfg!(target_os = "linux") {
            assert!(roots.iter().any(|r| r.ends_with("media")));
            assert!(roots.iter().any(|r| r == Path::new("/mnt")));
        }
    }

    #[test]
    fn percent_decoding_round_trips() {
        assert_eq!(percent_decode("/home/me/notes.txt"), "/home/me/notes.txt");
        assert_eq!(percent_decode("/home/me/My%20Notes.txt"), "/home/me/My Notes.txt");
        assert_eq!(percent_decode("/home/me/100%25.txt"), "/home/me/100%.txt");
        // A stray '%' is left alone rather than eating the next bytes.
        assert_eq!(percent_decode("/home/me/50%off"), "/home/me/50%off");
        // UTF-8 arrives percent-encoded byte by byte.
        assert_eq!(percent_decode("/home/me/Caf%C3%A9"), "/home/me/Café");
    }

    #[test]
    fn file_uris_become_paths() {
        assert_eq!(
            file_uri_to_path("file:///home/me/My%20Notes.txt"),
            Some(PathBuf::from("/home/me/My Notes.txt"))
        );
        assert_eq!(
            file_uri_to_path("file://localhost/home/me/x.txt"),
            Some(PathBuf::from("/home/me/x.txt"))
        );
        assert_eq!(file_uri_to_path("https://example.com/x"), None);
        assert_eq!(file_uri_to_path("file://relative"), None);
    }
}

//! What differs between macOS and Linux for this computer's files, in plain
//! functions so it can be unit-tested without Tauri (see AGENTS.md): the
//! folders volumes are mounted in, `%XX` decoding and `file://` URIs, the
//! XDG data folder, and the rename that refuses a taken name. The app's own
//! `platform.rs` (opening files, windows, installs) re-exports them.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

pub const IS_MACOS: bool = cfg!(target_os = "macos");

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
        let Ok(entries) = fs::read_dir(base) else {
            continue;
        };
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
pub fn is_mount_point(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Some(parent) = path.parent() else {
        return true;
    };
    match (fs::metadata(path), fs::metadata(parent)) {
        (Ok(a), Ok(b)) => a.dev() != b.dev(),
        _ => false,
    }
}

#[cfg(not(unix))]
pub fn is_mount_point(_: &Path) -> bool {
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
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
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
            found.push(Volume {
                name,
                path: path.to_string_lossy().to_string(),
            });
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

/// The `file://` URI of a local path, every byte outside RFC 3986's
/// unreserved and sub-delimiter set percent-encoded, as GLib's
/// `g_filename_to_uri` does. Unescaped, `#` would end the path and `%`
/// would be read as an escape by the app receiving a drag.
pub fn path_to_file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    const KEEP: &[u8] = b"-._~!$&'()*+,;=:@/";
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || KEEP.contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
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

/// `$XDG_DATA_HOME`, or its default `~/.local/share`. Used for the Trash and
/// the recently-used list on Linux.
pub fn data_home(home: &Path) -> PathBuf {
    data_home_from(Some(home), std::env::var_os("XDG_DATA_HOME")).unwrap_or_else(|| home.join(".local/share"))
}

/// `data_home` from the values of `HOME` and `XDG_DATA_HOME`; a relative
/// `XDG_DATA_HOME` is ignored, as the XDG spec says.
pub fn data_home_from(home: Option<&Path>, data_home: Option<OsString>) -> Option<PathBuf> {
    data_home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|home| home.join(".local/share")))
}

// ---- Renames that refuse a taken name -----------------------------------

/// Rename `from` to `to` in one step that fails with `AlreadyExists` when
/// something is at `to`, where plain `rename` would replace a file (or an
/// empty folder): `renameat2(RENAME_NOREPLACE)` on Linux,
/// `renamex_np(RENAME_EXCL)` on macOS. Where there is no such rename, it
/// fails as `cannot_refuse` tells apart; `placement::rename_new` then checks
/// the name itself.
#[cfg(target_os = "linux")]
pub fn rename_exclusive(from: &Path, to: &Path) -> std::io::Result<()> {
    let (from, to) = (c_path(from)?, c_path(to)?);
    // SAFETY: both are NUL-terminated paths that live through the call.
    let done = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    os_result(done)
}

#[cfg(target_os = "macos")]
pub fn rename_exclusive(from: &Path, to: &Path) -> std::io::Result<()> {
    let (from, to) = (c_path(from)?, c_path(to)?);
    // SAFETY: both are NUL-terminated paths that live through the call.
    let done = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    os_result(done)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn rename_exclusive(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Whether `rename_exclusive` failed only because the system or the
/// filesystem has no rename that refuses a taken name (some network and
/// FUSE filesystems answer `EINVAL` or `ENOTSUP`).
pub fn cannot_refuse(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::Unsupported
        || e.raw_os_error().is_some_and(|code| CANNOT_REFUSE.contains(&code))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const CANNOT_REFUSE: [i32; 4] = [libc::EINVAL, libc::ENOSYS, libc::ENOTSUP, libc::EOPNOTSUPP];
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const CANNOT_REFUSE: [i32; 0] = [];

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn c_path(path: &Path) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::ErrorKind::InvalidInput.into())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn os_result(done: libc::c_int) -> std::io::Result<()> {
    if done == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
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
        assert_eq!(
            percent_decode("/home/me/My%20Notes.txt"),
            "/home/me/My Notes.txt"
        );
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


    #[test]
    fn paths_become_escaped_file_uris() {
        let uri = |p: &str| path_to_file_uri(Path::new(p));
        assert_eq!(uri("/home/me/C# notes.txt"), "file:///home/me/C%23%20notes.txt");
        assert_eq!(uri("/home/me/100%.txt"), "file:///home/me/100%25.txt");
        // A name that already looks escaped stays itself.
        assert_eq!(uri("/home/me/50%20off.txt"), "file:///home/me/50%2520off.txt");
        assert_eq!(uri("/home/me/a?b"), "file:///home/me/a%3Fb");
        assert_eq!(uri("/home/me/Café"), "file:///home/me/Caf%C3%A9");
        assert_eq!(uri("/home/me/it's (1).txt"), "file:///home/me/it's%20(1).txt");
        // Every one of them reads back as the same path.
        for p in ["/home/me/C# notes.txt", "/home/me/100%.txt", "/home/me/50%20off.txt", "/home/me/Café", "/tmp/a\nb"] {
            assert_eq!(file_uri_to_path(&uri(p)), Some(PathBuf::from(p)), "{p}");
        }
    }

    // ---- renames that refuse a taken name ----

    /// The answers of a system or filesystem without a rename that refuses
    /// (`EINVAL` from some network filesystems) are told apart from a
    /// rename that failed for itself.
    #[test]
    fn a_rename_that_cannot_refuse_is_told_apart() {
        use std::io::{Error, ErrorKind};
        for code in [libc::EINVAL, libc::ENOSYS, libc::ENOTSUP, libc::EOPNOTSUPP] {
            assert!(cannot_refuse(&Error::from_raw_os_error(code)), "{code}");
        }
        assert!(cannot_refuse(&ErrorKind::Unsupported.into()));
        for code in [libc::EEXIST, libc::EACCES, libc::ENOENT, libc::EXDEV] {
            assert!(!cannot_refuse(&Error::from_raw_os_error(code)), "{code}");
        }
    }

    /// The real one, on this filesystem: a free name, then a taken file and
    /// an empty folder, which `rename` would both replace. A filesystem
    /// without it says so instead, and nothing moves.
    #[test]
    fn rename_exclusive_never_replaces() {
        let dir = tempdir().unwrap();
        let (from, to, empty) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("empty"));
        fs::write(&from, "mine").unwrap();
        fs::write(&to, "theirs").unwrap();
        fs::create_dir(&empty).unwrap();
        for taken in [&to, &empty] {
            let err = rename_exclusive(&from, taken).unwrap_err();
            assert!(err.kind() == std::io::ErrorKind::AlreadyExists || cannot_refuse(&err), "{err}");
        }
        assert_eq!(fs::read_to_string(&to).unwrap(), "theirs");
        assert_eq!(fs::read_to_string(&from).unwrap(), "mine");
        assert!(empty.is_dir());
        fs::remove_file(&to).unwrap();
        match rename_exclusive(&from, &to) {
            Ok(()) => assert_eq!(fs::read_to_string(&to).unwrap(), "mine"),
            Err(e) => assert!(cannot_refuse(&e), "{e}"),
        }
    }
}

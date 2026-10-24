//! Everything the Properties dialog shows: size, dates, permissions, link
//! targets and folder contents. Plain functions so they can be tested without
//! Tauri; `lib.rs` wraps the slow ones in the blocking-thread pool.

use serde::Serialize;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, PartialEq)]
pub struct FileProperties {
    pub name: String,
    pub path: String,
    /// The containing folder, or empty for a filesystem root.
    pub parent: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// Where a symlink points, if it is one.
    pub symlink_target: Option<String>,
    pub size: u64,
    /// Direct children for folders; `None` for files.
    pub item_count: Option<u32>,
    pub modified: i64,
    pub accessed: i64,
    /// Birth time; 0 when the filesystem doesn't report one.
    pub created: i64,
    /// Permissions in `ls -l` form, e.g. "drwxr-xr-x".
    pub mode: String,
    /// The same permission bits in octal, e.g. "755".
    pub octal: String,
    pub owner: String,
    pub group: String,
    pub others: String,
}

fn unix_secs(time: std::io::Result<SystemTime>) -> i64 {
    time.ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The nine permission bits (and setuid/setgid/sticky) as `ls -l` prints them.
pub fn permission_string(mode: u32) -> String {
    let kind = match (mode >> 12) & 0o17 {
        0o4 => 'd',
        0o12 => 'l',
        0o2 => 'c',
        0o6 => 'b',
        0o1 => 'p',
        0o14 => 's',
        _ => '-',
    };
    let mut out = String::with_capacity(10);
    out.push(kind);
    let specials = [(0o4000, 's', 'S'), (0o2000, 's', 'S'), (0o1000, 't', 'T')];
    for (i, &(special, on, off)) in specials.iter().enumerate() {
        let shift = 6 - (i as u32) * 3;
        out.push(if mode & (0o4 << shift) != 0 { 'r' } else { '-' });
        out.push(if mode & (0o2 << shift) != 0 { 'w' } else { '-' });
        let execute = mode & (0o1 << shift) != 0;
        out.push(if mode & special != 0 {
            if execute { on } else { off }
        } else if execute {
            'x'
        } else {
            '-'
        });
    }
    out
}

/// Permission bits of one class in GNOME Files' wording.
fn access_label(mode: u32, shift: u32) -> &'static str {
    let read = mode & (0o4 << shift) != 0;
    let write = mode & (0o2 << shift) != 0;
    let execute = mode & (0o1 << shift) != 0;
    match (read, write) {
        (true, true) => "Read and write",
        (true, false) => "Read-only",
        (false, true) => "Write-only",
        (false, false) if execute => "Execute-only",
        (false, false) => "None",
    }
}

pub fn file_properties(path: &str, show_hidden: bool) -> Result<FileProperties, String> {
    let p = Path::new(path);
    let link_meta = std::fs::symlink_metadata(p)
        .map_err(|e| format!("Could not read properties: {}", e))?;
    let is_symlink = link_meta.file_type().is_symlink();
    // Show the target's size and dates, like GNOME Files; a broken link keeps
    // what the link itself reports.
    let meta = if is_symlink {
        std::fs::metadata(p).unwrap_or_else(|_| link_meta.clone())
    } else {
        link_meta
    };

    #[cfg(unix)]
    let mode_bits = {
        use std::os::unix::fs::MetadataExt;
        meta.mode()
    };
    #[cfg(not(unix))]
    let mode_bits = if meta.is_dir() { 0o40755 } else { 0o100644 };

    let is_dir = meta.is_dir();
    let bytes = meta.len();
    let mode = permission_string(mode_bits);

    Ok(FileProperties {
        name: p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string()),
        path: path.to_string(),
        parent: p
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.to_string_lossy().to_string())
            .unwrap_or_default(),
        is_dir,
        is_symlink,
        symlink_target: if is_symlink {
            std::fs::read_link(p)
                .ok()
                .map(|target| target.to_string_lossy().to_string())
        } else {
            None
        },
        size: bytes,
        item_count: if is_dir {
            crate::fs_ops::count_items(p, show_hidden)
        } else {
            None
        },
        modified: unix_secs(meta.modified()),
        accessed: unix_secs(meta.accessed()),
        created: unix_secs(meta.created()),
        octal: format!("{:o}", mode_bits & 0o7777),
        owner: access_label(mode_bits, 6).to_string(),
        group: access_label(mode_bits, 3).to_string(),
        others: access_label(mode_bits, 0).to_string(),
        mode,
    })
}

/// Total size of every file inside a folder, recursively. Symlinks are not
/// followed (so links can't loop); an unreadable folder shows as unknown
/// rather than as empty, while unreadable entries inside are skipped.
pub fn folder_size(path: &str) -> Result<u64, String> {
    let p = Path::new(path);
    std::fs::read_dir(p).map_err(|e| format!("Could not read folder: {}", e))?;

    // Iterative rather than recursive: deeply nested trees can't overflow the stack.
    let mut total = 0;
    let mut pending = vec![p.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if meta.is_dir() {
                pending.push(entry.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn prints_permissions_like_ls() {
        assert_eq!(permission_string(0o40755), "drwxr-xr-x");
        assert_eq!(permission_string(0o100644), "-rw-r--r--");
        assert_eq!(permission_string(0o100600), "-rw-------");
        assert_eq!(permission_string(0o104755), "-rwsr-xr-x");
        assert_eq!(permission_string(0o102644), "-rw-r-Sr--");
        assert_eq!(permission_string(0o41777), "drwxrwxrwt");
        assert_eq!(permission_string(0o120777), "lrwxrwxrwx");
    }

    #[test]
    fn describes_a_file() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("hello.txt");
        fs::write(&file, "world").unwrap();
        let path = file.to_string_lossy().to_string();

        let props = file_properties(&path, true).unwrap();
        assert_eq!(props.name, "hello.txt");
        assert_eq!(props.path, path);
        assert_eq!(props.parent, dir.path().to_string_lossy());
        assert!(!props.is_dir);
        assert!(!props.is_symlink);
        assert_eq!(props.symlink_target, None);
        assert_eq!(props.size, 5);
        assert_eq!(props.item_count, None);
        assert!(props.modified > 0);
        assert!(props.accessed > 0);
        assert!(props.mode.starts_with('-'));
        assert_eq!(props.mode.len(), 10);
        assert!(props.octal.len() >= 3);
        assert_eq!(props.owner, "Read and write");
    }

    #[test]
    fn describes_a_folder_and_counts_visible_items() {
        let dir = tempdir().unwrap();
        let folder = dir.path().join("Folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("visible.txt"), "x").unwrap();
        fs::write(folder.join(".hidden"), "x").unwrap();

        let path = folder.to_string_lossy().to_string();
        let props = file_properties(&path, false).unwrap();
        assert!(props.is_dir);
        assert_eq!(props.item_count, Some(1));
        assert!(props.mode.starts_with('d'));
        assert!(props.octal.len() >= 3);

        assert_eq!(file_properties(&path, true).unwrap().item_count, Some(2));
    }

    #[cfg(unix)]
    #[test]
    fn describes_a_symlink_target() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.txt");
        fs::write(&target, "hello").unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let props = file_properties(&link.to_string_lossy(), true).unwrap();
        assert!(props.is_symlink);
        assert!(!props.is_dir);
        assert_eq!(props.symlink_target, Some(target.to_string_lossy().to_string()));
        assert_eq!(props.size, 5);
    }

    #[test]
    fn missing_paths_are_an_error() {
        let dir = tempdir().unwrap();
        let gone = dir.path().join("gone.txt");
        assert!(file_properties(&gone.to_string_lossy(), true).is_err());
    }

    #[test]
    fn sums_folder_sizes_without_following_links() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("a.txt"), "12345").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub/b.txt"), "123").unwrap();

        assert_eq!(folder_size(&root.to_string_lossy()), Ok(8));

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("a.txt"), root.join("link")).unwrap();
            // The link is not followed, so the total is unchanged.
            assert_eq!(folder_size(&root.to_string_lossy()), Ok(8));
        }
    }

    #[test]
    fn folder_size_of_a_missing_folder_is_an_error() {
        let dir = tempdir().unwrap();
        assert!(folder_size(&dir.path().join("gone").to_string_lossy()).is_err());
    }
}

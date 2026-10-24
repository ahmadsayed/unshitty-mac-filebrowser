//! Everything the Properties dialog shows: size, dates, permissions, link
//! targets and folder contents. Plain functions so they can be tested without
//! Tauri; `lib.rs` wraps the slow ones in the blocking-thread pool.

use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
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

/// What one class (owner, group or others) may do, in GNOME Files' wording.
/// Folders get their own labels, because there the execute bit decides
/// whether the files inside can be reached at all; for files it is ignored.
pub fn access_label(mode: u32, shift: u32, is_dir: bool) -> &'static str {
    let read = mode & (0o4 << shift) != 0;
    let write = mode & (0o2 << shift) != 0;
    let execute = mode & (0o1 << shift) != 0;
    if !is_dir {
        return match (read, write) {
            (true, true) => "Read and write",
            (true, false) => "Read-only",
            (false, true) => "Write-only",
            (false, false) => "None",
        };
    }
    match (read, write, execute) {
        (true, true, true) => "Create and delete files",
        (true, true, false) => "Read/write, no access",
        (true, false, true) => "Access files",
        (true, false, false) => "List files only",
        (false, true, true) => crate::platform::WRITE_ONLY_FOLDER,
        (false, true, false) => "Write-only, no access",
        (false, false, true) => "Access-only",
        (false, false, false) => "None",
    }
}

/// The permission bits in octal: three digits, or four when setuid, setgid
/// or sticky is set ("644", "1777").
fn octal_string(mode: u32) -> String {
    if mode & 0o7000 != 0 {
        format!("{:04o}", mode & 0o7777)
    } else {
        format!("{:03o}", mode & 0o777)
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
        octal: octal_string(mode_bits),
        owner: access_label(mode_bits, 6, is_dir).to_string(),
        group: access_label(mode_bits, 3, is_dir).to_string(),
        others: access_label(mode_bits, 0, is_dir).to_string(),
        mode,
    })
}

/// Folder-size walks with an id up to this one are cancelled. The frontend
/// numbers its walks in increasing order (from the clock, so the order holds
/// across reloads), so one number covers every walk it has given up on, even
/// a cancel that overtakes its own walk on the way in.
static CANCELLED_UP_TO: AtomicU64 = AtomicU64::new(0);

/// Stop walk `id` and every earlier one.
pub fn cancel_walks_up_to(id: u64) {
    CANCELLED_UP_TO.fetch_max(id, Ordering::Relaxed);
}

pub fn walk_cancelled(id: u64) -> bool {
    id <= CANCELLED_UP_TO.load(Ordering::Relaxed)
}

/// Total size of every file inside a folder, recursively, counted the way
/// GNOME Files does: symlinks are not followed (so links can't loop), other
/// filesystems mounted inside are not entered, and a file with several hard
/// links is counted once. An unreadable folder shows as unknown rather than
/// as empty, while unreadable entries inside are skipped. Stops with an error
/// as soon as `cancelled` returns true.
pub fn folder_size(path: &str, cancelled: impl Fn() -> bool) -> Result<u64, String> {
    let p = Path::new(path);
    let root = std::fs::metadata(p).map_err(|e| format!("Could not read folder: {}", e))?;
    std::fs::read_dir(p).map_err(|e| format!("Could not read folder: {}", e))?;
    let device = file_id(&root).map(|id| id.device);
    let mut seen_links: HashSet<(u64, u64)> = HashSet::new();

    // Iterative rather than recursive: deeply nested trees can't overflow the stack.
    let mut total = 0;
    let mut pending = vec![p.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if cancelled() {
                return Err("Cancelled".to_string());
            }
            // `DirEntry::metadata` does not follow symlinks.
            let Ok(meta) = entry.metadata() else { continue };
            if let Some(id) = file_id(&meta) {
                if Some(id.device) != device {
                    continue;
                }
                if meta.is_file() && id.links > 1 && !seen_links.insert((id.device, id.inode)) {
                    continue;
                }
            }
            if meta.is_dir() {
                pending.push(entry.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    Ok(total)
}

struct FileId {
    device: u64,
    inode: u64,
    links: u64,
}

/// Where a file lives on disk, for spotting mount points and hard links;
/// `None` on platforms that don't report it.
#[cfg(unix)]
fn file_id(meta: &std::fs::Metadata) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    Some(FileId { device: meta.dev(), inode: meta.ino(), links: meta.nlink() })
}

#[cfg(not(unix))]
fn file_id(_meta: &std::fs::Metadata) -> Option<FileId> {
    None
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
    fn labels_access_like_gnome_files() {
        // Files: the execute bit doesn't change the label.
        assert_eq!(access_label(0o600, 6, false), "Read and write");
        assert_eq!(access_label(0o700, 6, false), "Read and write");
        assert_eq!(access_label(0o040, 3, false), "Read-only");
        assert_eq!(access_label(0o200, 6, false), "Write-only");
        assert_eq!(access_label(0o001, 0, false), "None");
        assert_eq!(access_label(0o000, 0, false), "None");

        // Folders: what each class can do with the files inside.
        assert_eq!(access_label(0o700, 6, true), "Create and delete files");
        assert_eq!(access_label(0o600, 6, true), "Read/write, no access");
        assert_eq!(access_label(0o050, 3, true), "Access files");
        assert_eq!(access_label(0o004, 0, true), "List files only");
        assert_eq!(access_label(0o003, 0, true), crate::platform::WRITE_ONLY_FOLDER);
        assert_eq!(access_label(0o002, 0, true), "Write-only, no access");
        assert_eq!(access_label(0o001, 0, true), "Access-only");
        assert_eq!(access_label(0o000, 0, true), "None");
    }

    #[test]
    fn prints_octal_with_leading_zeros() {
        assert_eq!(octal_string(0o100644), "644");
        assert_eq!(octal_string(0o100044), "044");
        assert_eq!(octal_string(0o100000), "000");
        assert_eq!(octal_string(0o41777), "1777");
        assert_eq!(octal_string(0o104755), "4755");
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
        assert_eq!(props.mode, permission_string(0o100000 | u32::from_str_radix(&props.octal, 8).unwrap()));
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
        assert_eq!(props.owner, "Create and delete files");

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

        assert_eq!(folder_size(&root.to_string_lossy(), || false), Ok(8));

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("a.txt"), root.join("link")).unwrap();
            // The link is not followed, so the total is unchanged.
            assert_eq!(folder_size(&root.to_string_lossy(), || false), Ok(8));
        }
    }

    #[test]
    fn counts_hard_linked_files_once() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.txt"), "12345").unwrap();
        fs::hard_link(root.join("a.txt"), root.join("sub/a-again.txt")).unwrap();
        fs::write(root.join("b.txt"), "123").unwrap();

        assert_eq!(folder_size(&root.to_string_lossy(), || false), Ok(8));
    }

    #[test]
    fn folder_size_stops_when_cancelled() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "12345").unwrap();
        assert!(folder_size(&dir.path().to_string_lossy(), || true).is_err());
    }

    #[test]
    fn cancelling_a_walk_cancels_every_earlier_one() {
        // Ids far above what other tests or the clock use, so this can't
        // cancel them; `CANCELLED_UP_TO` is shared by the whole process.
        let base = u64::MAX - 100;
        assert!(!walk_cancelled(base + 2));
        cancel_walks_up_to(base + 2);
        assert!(walk_cancelled(base + 1));
        assert!(walk_cancelled(base + 2));
        assert!(!walk_cancelled(base + 3));
        // An older cancel arriving late doesn't un-cancel anything.
        cancel_walks_up_to(base);
        assert!(walk_cancelled(base + 2));
    }

    #[test]
    fn folder_size_of_a_missing_folder_is_an_error() {
        let dir = tempdir().unwrap();
        assert!(folder_size(&dir.path().join("gone").to_string_lossy(), || false).is_err());
    }
}

use std::path::{Path, PathBuf};
use serde::Serialize;

#[tauri::command]
pub fn make_dir(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if p.exists() {
        return Err(format!("Path already exists: {}", path));
    }
    std::fs::create_dir(p).map_err(|e| format!("Failed to create directory: {}", e))
}

#[tauri::command]
pub fn rename_entry(from: String, to: String) -> Result<(), String> {
    let to_path = Path::new(&to);
    if to_path.exists() {
        return Err(format!("Target already exists: {}", to));
    }
    std::fs::rename(&from, &to).map_err(|e| format!("Failed to rename: {}", e))
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: i64,
    pub is_hidden: bool,
    pub is_symlink: bool,
}

pub fn read_entries(path: &Path, show_hidden: bool) -> Result<Vec<Entry>, String> {
    use std::time::UNIX_EPOCH;

    let read_dir = std::fs::read_dir(path)
        .map_err(|e| format!("Failed to read directory: {}", e))?;

    let mut entries = Vec::new();

    for dir_entry_result in read_dir {
        let dir_entry = match dir_entry_result {
            Ok(e) => e,
            Err(_) => continue,
        };

        let name = dir_entry.file_name().to_string_lossy().to_string();

        let is_hidden = name.starts_with('.');

        if is_hidden && !show_hidden {
            continue;
        }

        let entry_path = dir_entry.path();
        let path_str = entry_path.to_string_lossy().to_string();

        // Use symlink_metadata so broken symlinks don't error the whole listing
        let symlink_meta = match std::fs::symlink_metadata(&entry_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let is_symlink = symlink_meta.file_type().is_symlink();

        // For is_dir and size, follow the link if it's a symlink (use regular metadata)
        let (is_dir, size, modified) = if is_symlink {
            // Try to follow the link for metadata; if broken, use defaults
            match std::fs::metadata(&entry_path) {
                Ok(meta) => {
                    let modified = meta.modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    (meta.is_dir(), meta.len(), modified)
                }
                Err(_) => {
                    // Broken symlink: use symlink_meta for what we can
                    let modified = symlink_meta.modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    (false, symlink_meta.len(), modified)
                }
            }
        } else {
            let modified = symlink_meta.modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            (symlink_meta.is_dir(), symlink_meta.len(), modified)
        };

        entries.push(Entry {
            name,
            path: path_str,
            is_dir,
            size,
            modified,
            is_hidden,
            is_symlink,
        });
    }

    Ok(entries)
}

// Commands that touch many files are `async` so Tauri runs them off the main
// thread and the window stays responsive during big listings or copies.
#[tauri::command(async)]
pub fn list_dir(path: String, show_hidden: bool) -> Result<Vec<Entry>, String> {
    read_entries(Path::new(&path), show_hidden)
}

/// Number of entries directly inside `path`, or `None` if it can't be read.
pub fn count_items(path: &Path, show_hidden: bool) -> Option<u32> {
    let read_dir = std::fs::read_dir(path).ok()?;
    let count = read_dir
        .filter_map(|e| e.ok())
        .filter(|e| show_hidden || !e.file_name().to_string_lossy().starts_with('.'))
        .count();
    Some(count as u32)
}

/// Item counts for several folders at once (the "12 items" size column).
#[tauri::command(async)]
pub fn item_counts(paths: Vec<String>, show_hidden: bool) -> Vec<Option<u32>> {
    paths
        .iter()
        .map(|p| count_items(Path::new(p), show_hidden))
        .collect()
}

// ---------------------------------------------------------------------------
// Copy / move / trash
//
// Nothing here ever deletes data outright except the source of a finished
// cross-volume move. A replaced target goes to the Trash, and the new item is
// staged under a temporary name first so a failed copy leaves the target alone.
// ---------------------------------------------------------------------------

/// True if something (even a broken symlink) exists at `path`.
#[tauri::command]
pub fn path_exists(path: String) -> bool {
    std::fs::symlink_metadata(&path).is_ok()
}

/// Absolute, symlink-resolved form of `path`, which may not exist yet.
fn resolve(path: &Path) -> PathBuf {
    if let Ok(p) = path.canonicalize() {
        return p;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => resolve(parent).join(name),
        _ => path.to_path_buf(),
    }
}

/// True when `child` is `ancestor` itself or lies anywhere inside it.
pub fn is_subpath(child: &Path, ancestor: &Path) -> bool {
    resolve(child).starts_with(resolve(ancestor))
}

/// Copy a file, symlink or whole directory tree. Symlinks are recreated, not
/// followed, so a link pointing at a parent can't cause an endless copy.
pub fn copy_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)?;
        #[cfg(not(unix))]
        std::fs::copy(from, to)?;
    } else if meta.is_dir() {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
        std::fs::set_permissions(to, meta.permissions())?;
    } else {
        std::fs::copy(from, to)?;
    }
    Ok(())
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// A free hidden name next to `to`, used to stage a copy before it replaces `to`.
fn staging_path(to: &Path) -> PathBuf {
    let dir = to.parent().unwrap_or(Path::new("/"));
    let name = to.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut n = 0u32;
    loop {
        let candidate = dir.join(format!(".{}.filebrowser-{}-{}", name, std::process::id(), n));
        if std::fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
        n += 1;
    }
}

/// Shared refusals for copy and move, checked before anything is written.
fn check_transfer(from: &Path, to: &Path, overwrite: bool) -> Result<bool, String> {
    if std::fs::symlink_metadata(from).is_err() {
        return Err(format!("“{}” no longer exists", from.display()));
    }
    if resolve(from) == resolve(to) {
        return Err("The source and destination are the same".to_string());
    }
    if is_subpath(to, from) {
        return Err("Cannot copy or move a folder into itself".to_string());
    }
    let to_exists = std::fs::symlink_metadata(to).is_ok();
    if to_exists {
        if !overwrite {
            return Err(format!("Target already exists: {}", to.display()));
        }
        if is_subpath(from, to) {
            return Err("Cannot replace a folder with an item inside it".to_string());
        }
    }
    Ok(to_exists)
}

/// Swap the staged item into place, sending whatever was at `to` to the Trash.
fn replace_with_staged(staged: &Path, to: &Path) -> Result<(), String> {
    if let Err(e) = trash::delete(to) {
        let _ = remove_any(staged);
        return Err(format!("Could not move the existing item to the Trash: {}", e));
    }
    std::fs::rename(staged, to).map_err(|e| format!("Failed to replace: {}", e))
}

fn is_cross_device(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::CrossesDevices || e.raw_os_error() == Some(18) // EXDEV
}

/// Copy `from` to `to` (recursively for folders).
#[tauri::command(async)]
pub fn copy_entry(from: String, to: String, overwrite: bool) -> Result<(), String> {
    let (from, to) = (Path::new(&from), Path::new(&to));
    let replacing = check_transfer(from, to, overwrite)?;
    let dest = if replacing { staging_path(to) } else { to.to_path_buf() };
    if let Err(e) = copy_recursive(from, &dest) {
        let _ = remove_any(&dest);
        return Err(format!("Failed to copy: {}", e));
    }
    if replacing {
        replace_with_staged(&dest, to)?;
    }
    Ok(())
}

/// Move `from` to `to`: a rename on the same volume, copy-then-delete across volumes.
#[tauri::command(async)]
pub fn move_entry(from: String, to: String, overwrite: bool) -> Result<(), String> {
    let (from, to) = (Path::new(&from), Path::new(&to));
    let replacing = check_transfer(from, to, overwrite)?;
    let dest = if replacing { staging_path(to) } else { to.to_path_buf() };
    match std::fs::rename(from, &dest) {
        Ok(()) => {}
        Err(e) if is_cross_device(&e) => {
            if let Err(e) = copy_recursive(from, &dest) {
                let _ = remove_any(&dest);
                return Err(format!("Failed to move: {}", e));
            }
            if replacing {
                replace_with_staged(&dest, to)?;
            }
            // Only now, with the copy complete, is the source removed.
            return remove_any(from).map_err(|e| format!("Moved, but could not remove the original: {}", e));
        }
        Err(e) => return Err(format!("Failed to move: {}", e)),
    }
    if replacing {
        replace_with_staged(&dest, to)?;
    }
    Ok(())
}

/// Move `path` to the system Trash (recoverable).
#[tauri::command(async)]
pub fn trash_entry(path: String) -> Result<(), String> {
    if std::fs::symlink_metadata(&path).is_err() {
        return Err(format!("“{}” no longer exists", path));
    }
    trash::delete(&path).map_err(|e| format!("Failed to move to Trash: {}", e))
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn lists_file_and_subdir() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("hello.txt");
        fs::write(&file_path, "world").unwrap();
        let sub_path = dir.path().join("subdir");
        fs::create_dir(&sub_path).unwrap();

        let entries = read_entries(dir.path(), true).unwrap();

        let file_entry = entries.iter().find(|e| e.name == "hello.txt").expect("hello.txt not found");
        assert!(!file_entry.is_dir);
        assert_eq!(file_entry.size, 5);

        let dir_entry = entries.iter().find(|e| e.name == "subdir").expect("subdir not found");
        assert!(dir_entry.is_dir);
    }

    #[test]
    fn dotfile_excluded_when_hidden_false() {
        let dir = tempdir().unwrap();
        let dotfile = dir.path().join(".hidden");
        fs::write(&dotfile, "secret").unwrap();

        let entries = read_entries(dir.path(), false).unwrap();
        assert!(!entries.iter().any(|e| e.name == ".hidden"));
    }

    #[test]
    fn dotfile_included_when_hidden_true_and_marked_hidden() {
        let dir = tempdir().unwrap();
        let dotfile = dir.path().join(".hidden");
        fs::write(&dotfile, "secret").unwrap();

        let entries = read_entries(dir.path(), true).unwrap();
        let entry = entries.iter().find(|e| e.name == ".hidden").expect(".hidden not found");
        assert!(entry.is_hidden);
    }

    #[test]
    fn nonexistent_path_returns_err() {
        let result = read_entries(Path::new("/no/such/path/exists/here"), false);
        assert!(result.is_err());
    }

    #[test]
    fn broken_symlink_included_as_symlink() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("missing_target");
        let link = dir.path().join("broken_link");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();

        #[cfg(unix)]
        {
            let entries = read_entries(dir.path(), true).unwrap();
            let entry = entries.iter().find(|e| e.name == "broken_link").expect("broken_link not found");
            assert!(entry.is_symlink);
        }
    }

    // ----- item_counts tests -----

    #[test]
    fn count_items_respects_hidden_flag() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join(".hidden"), "h").unwrap();

        assert_eq!(count_items(dir.path(), false), Some(2));
        assert_eq!(count_items(dir.path(), true), Some(3));
    }

    #[test]
    fn item_counts_returns_none_for_unreadable_path() {
        let dir = tempdir().unwrap();
        let counts = item_counts(
            vec![
                dir.path().to_string_lossy().to_string(),
                "/no/such/path/exists/here".to_string(),
            ],
            false,
        );
        assert_eq!(counts, vec![Some(0), None]);
    }

    // ----- make_dir tests -----

    #[test]
    fn make_dir_creates_directory() {
        let dir = tempdir().unwrap();
        let new_dir = dir.path().join("new_folder");
        let path_str = new_dir.to_string_lossy().to_string();

        assert!(!new_dir.exists());
        make_dir(path_str).unwrap();
        assert!(new_dir.exists());
        assert!(new_dir.is_dir());
    }

    #[test]
    fn make_dir_errors_when_already_exists() {
        let dir = tempdir().unwrap();
        let existing = dir.path().join("existing_folder");
        fs::create_dir(&existing).unwrap();
        let path_str = existing.to_string_lossy().to_string();

        let result = make_dir(path_str);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("already exists"), "unexpected error: {}", msg);
    }

    // ----- rename_entry tests -----

    #[test]
    fn rename_entry_renames_file() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("original.txt");
        let dst = dir.path().join("renamed.txt");
        fs::write(&src, "hello").unwrap();

        rename_entry(
            src.to_string_lossy().to_string(),
            dst.to_string_lossy().to_string(),
        )
        .unwrap();

        assert!(!src.exists());
        assert!(dst.exists());
    }

    #[test]
    fn rename_entry_errors_when_target_exists() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("target.txt");
        fs::write(&src, "hello").unwrap();
        fs::write(&dst, "world").unwrap();

        let result = rename_entry(
            src.to_string_lossy().to_string(),
            dst.to_string_lossy().to_string(),
        );
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("already exists"), "unexpected error: {}", msg);
    }

    // ----- copy / move / trash tests -----

    fn s(p: &Path) -> String {
        p.to_string_lossy().to_string()
    }

    #[test]
    fn copy_entry_copies_file() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "hello").unwrap();

        copy_entry(s(&src), s(&dst), false).unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "hello");
        assert!(src.exists());
    }

    #[test]
    fn copy_entry_copies_dir_tree() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("tree");
        fs::create_dir_all(src.join("sub/deeper")).unwrap();
        fs::write(src.join("top.txt"), "1").unwrap();
        fs::write(src.join("sub/deeper/leaf.txt"), "2").unwrap();
        let dst = dir.path().join("copy");

        copy_entry(s(&src), s(&dst), false).unwrap();
        assert_eq!(fs::read_to_string(dst.join("top.txt")).unwrap(), "1");
        assert_eq!(fs::read_to_string(dst.join("sub/deeper/leaf.txt")).unwrap(), "2");
    }

    #[cfg(unix)]
    #[test]
    fn copy_entry_recreates_symlinks_instead_of_following() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("tree");
        fs::create_dir(&src).unwrap();
        // A link back to its own parent would recurse forever if followed.
        std::os::unix::fs::symlink(&src, src.join("loop")).unwrap();
        let dst = dir.path().join("copy");

        copy_entry(s(&src), s(&dst), false).unwrap();
        let meta = fs::symlink_metadata(dst.join("loop")).unwrap();
        assert!(meta.file_type().is_symlink());
    }

    #[test]
    fn copy_entry_refuses_existing_target_without_overwrite() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        let err = copy_entry(s(&src), s(&dst), false).unwrap_err();
        assert!(err.contains("already exists"), "unexpected error: {}", err);
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
    }

    #[test]
    fn copy_entry_overwrites_when_asked() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        copy_entry(s(&src), s(&dst), true).unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "new");
        // No staging leftovers in the folder.
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 2, "unexpected entries: {:?}", names);
    }

    #[test]
    fn is_subpath_detects_nesting() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        fs::create_dir(&a).unwrap();
        assert!(is_subpath(&a.join("b/c"), &a));
        assert!(is_subpath(&a, &a));
        assert!(!is_subpath(&dir.path().join("ab"), &a));
        assert!(!is_subpath(dir.path(), &a));
    }

    #[test]
    fn copy_and_move_refuse_into_itself() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        fs::create_dir(&a).unwrap();
        let inside = a.join("a");

        assert!(copy_entry(s(&a), s(&inside), false).is_err());
        assert!(move_entry(s(&a), s(&inside), false).is_err());
        assert!(!inside.exists());
    }

    #[test]
    fn copy_and_move_refuse_same_path() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("f.txt");
        fs::write(&f, "x").unwrap();

        assert!(copy_entry(s(&f), s(&f), true).is_err());
        assert!(move_entry(s(&f), s(&f), true).is_err());
        assert_eq!(fs::read_to_string(&f).unwrap(), "x");
    }

    #[test]
    fn replacing_a_folder_with_its_own_child_is_refused() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("p");
        fs::create_dir_all(parent.join("child")).unwrap();

        assert!(copy_entry(s(&parent.join("child")), s(&parent), true).is_err());
        assert!(move_entry(s(&parent.join("child")), s(&parent), true).is_err());
        assert!(parent.join("child").exists());
    }

    #[test]
    fn move_entry_moves_file_and_removes_source() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        fs::create_dir(dir.path().join("dest")).unwrap();
        let dst = dir.path().join("dest/a.txt");
        fs::write(&src, "hello").unwrap();

        move_entry(s(&src), s(&dst), false).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "hello");
    }

    #[test]
    fn move_entry_refuses_existing_target_without_overwrite() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        assert!(move_entry(s(&src), s(&dst), false).is_err());
        assert!(src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
    }

    #[test]
    fn move_entry_overwrites_when_asked() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        move_entry(s(&src), s(&dst), true).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "new");
    }

    #[test]
    fn trash_entry_removes_file_from_its_location() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("trash-me.txt");
        fs::write(&f, "bye").unwrap();

        trash_entry(s(&f)).unwrap();
        assert!(!f.exists());
    }

    #[test]
    fn operations_on_vanished_path_return_err() {
        let dir = tempdir().unwrap();
        let gone = dir.path().join("gone");
        let dst = dir.path().join("dst");

        assert!(copy_entry(s(&gone), s(&dst), false).is_err());
        assert!(move_entry(s(&gone), s(&dst), false).is_err());
        assert!(trash_entry(s(&gone)).is_err());
        assert!(!path_exists(s(&gone)));
    }
}

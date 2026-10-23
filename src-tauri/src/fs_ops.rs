use std::path::Path;
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

#[tauri::command]
pub fn list_dir(path: String, show_hidden: bool) -> Result<Vec<Entry>, String> {
    read_entries(Path::new(&path), show_hidden)
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
}

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
pub const WRITE_ONLY_FOLDER: &str = if IS_MACOS {
    "Write-only (Drop Box)"
} else {
    "Write-only"
};

/// Wait for `child` to exit, killing it once `timeout` has passed. True when it
/// exited on its own. External helpers (qlmanage, mdfind) can hang on unusual
/// input, and a stuck one must never hold a worker thread forever.
pub fn wait_or_kill(child: &mut std::process::Child, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Start an external helper (qlmanage, mdfind). A spawn that fails with
/// "text file busy" (ETXTBSY) is retried briefly: it happens when another
/// thread forks while the program file is still open for writing, which the
/// tests' stand-in scripts hit when the suite runs in parallel.
pub fn spawn_helper(command: &mut std::process::Command) -> std::io::Result<std::process::Child> {
    let mut attempt = 0;
    loop {
        match command.spawn() {
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 20 =>
            {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            result => return result,
        }
    }
}

/// A mounted volume as the sidebar shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Volume {
    /// Label: the folder name the mount point has.
    pub name: String,
    pub path: String,
}

/// An application offered by "Open With…".
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppChoice {
    pub name: String,
    /// macOS: the `.app` path; Linux: the `.desktop` file path.
    pub id: String,
}

/// Every application that can be offered by "Open With…".
pub fn applications() -> Vec<AppChoice> {
    let mut apps = if IS_MACOS {
        mac_applications()
    } else if cfg!(target_os = "linux") {
        linux_applications()
    } else {
        Vec::new()
    };
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps.dedup_by(|a, b| a.id == b.id);
    apps
}

fn mac_applications() -> Vec<AppChoice> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    let mut apps = Vec::new();
    for root in roots {
        collect_apps(&root, &mut apps, 0);
    }
    apps
}

fn collect_apps(dir: &Path, apps: &mut Vec<AppChoice>, depth: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("app") {
            let label = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| name.clone());
            apps.push(AppChoice {
                name: label,
                id: path.to_string_lossy().to_string(),
            });
        } else if depth == 0 && name == "Utilities" && path.is_dir() {
            collect_apps(&path, apps, depth + 1);
        }
    }
}

fn linux_applications() -> Vec<AppChoice> {
    let mut apps = Vec::new();
    for dir in linux_app_dirs() {
        apps.extend(apps_in_dir(&dir));
    }
    apps
}

fn linux_app_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/applications"));
        dirs.push(PathBuf::from(home).join(".local/share/flatpak/exports/share/applications"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for dir in data_dirs.split(':').filter(|dir| !dir.is_empty()) {
        dirs.push(PathBuf::from(dir).join("applications"));
    }
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs
}

fn apps_in_dir(dir: &Path) -> Vec<AppChoice> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("desktop"))
        .filter_map(|path| desktop_app(&path))
        .collect()
}

/// Parse the parts of a `.desktop` file that matter for Open With.
fn desktop_app(path: &Path) -> Option<AppChoice> {
    let text = fs::read_to_string(path).ok()?;
    let mut name = None;
    let mut kind = None;
    let mut hidden = false;
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(value) = line.strip_prefix("Name=") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("Type=") {
            kind = Some(value.trim().to_string());
        } else if line == "NoDisplay=true" || line == "Hidden=true" {
            hidden = true;
        }
    }
    if hidden || kind.as_deref() != Some("Application") {
        return None;
    }
    let name = name.filter(|name| !name.is_empty())?;
    Some(AppChoice {
        name,
        id: path.to_string_lossy().to_string(),
    })
}

/// Launch `path` with the default application for its type.
pub fn open_default(path: &str) -> Result<(), String> {
    fs::metadata(path).map_err(|e| e.to_string())?;
    let mut command = default_open_command(path)?;
    spawn_helper(&mut command)
        .map(|_| ())
        .map_err(|e| format!("Could not open with the default application: {e}"))
}

fn default_open_command(path: &str) -> Result<std::process::Command, String> {
    let mut command = if IS_MACOS {
        std::process::Command::new("open")
    } else if cfg!(target_os = "linux") {
        let mut command = std::process::Command::new("gio");
        command.arg("open");
        command
    } else {
        return Err("Opening files is not supported on this platform.".into());
    };
    command.arg(path);
    Ok(command)
}

/// Launch `path` with the chosen application.
pub fn open_with(path: &str, app: &str) -> Result<(), String> {
    if IS_MACOS {
        mac_open_with(path, app)
    } else if cfg!(target_os = "linux") {
        linux_open_with(path, app)
    } else {
        Err("Open With is not supported on this platform.".into())
    }
}

fn mac_open_with(path: &str, app: &str) -> Result<(), String> {
    let app_path = Path::new(app);
    if !app_path.is_dir() {
        return Err("That application is no longer available.".into());
    }
    std::process::Command::new("open")
        .arg("-a")
        .arg(app_path)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open with that application: {e}"))
}

fn linux_open_with(path: &str, app: &str) -> Result<(), String> {
    let desktop = Path::new(app);
    if !desktop.is_file() {
        return Err("That application is no longer available.".into());
    }
    if std::process::Command::new("gio")
        .arg("launch")
        .arg(desktop)
        .arg(path)
        .spawn()
        .is_ok()
    {
        return Ok(());
    }
    let id = desktop
        .file_name()
        .map(|name| name.to_string_lossy().trim_end_matches(".desktop").to_string())
        .unwrap_or_default();
    std::process::Command::new("gtk-launch")
        .arg(id)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open with that application: {e}"))
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
fn is_mount_point(path: &Path) -> bool {
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

/// Environment defaults for the webview, set before it starts.
///
/// WebKitGTK's DMA-BUF renderer makes popovers and dialogs flicker on some
/// GPU setups (seen on Intel + NVIDIA under Wayland); its shared-memory
/// fallback is smooth and fast enough for a file browser. Linux only.
pub const WEBVIEW_ENV: &[(&str, &str)] = if cfg!(target_os = "linux") {
    &[("WEBKIT_DISABLE_DMABUF_RENDERER", "1")]
} else {
    &[]
};

/// The `WEBVIEW_ENV` entries to set: those the user hasn't set already, so
/// exporting `WEBKIT_DISABLE_DMABUF_RENDERER=0` turns the renderer back on.
pub fn webview_env_to_set(is_set: impl Fn(&str) -> bool) -> Vec<(&'static str, &'static str)> {
    WEBVIEW_ENV
        .iter()
        .copied()
        .filter(|(k, _)| !is_set(k))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn webview_env_keeps_values_the_user_set() {
        assert_eq!(webview_env_to_set(|_| false), WEBVIEW_ENV.to_vec());
        assert!(webview_env_to_set(|_| true).is_empty());
        if cfg!(target_os = "linux") {
            assert_eq!(WEBVIEW_ENV, &[("WEBKIT_DISABLE_DMABUF_RENDERER", "1")]);
        } else {
            assert!(WEBVIEW_ENV.is_empty());
        }
    }

    #[test]
    fn mac_app_collection_finds_utilities_and_skips_hidden() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("TextEdit.app")).unwrap();
        fs::create_dir_all(dir.path().join("Utilities/Calculator.app")).unwrap();
        fs::create_dir_all(dir.path().join(".Hidden.app")).unwrap();
        fs::write(dir.path().join("notes.txt"), "x").unwrap();
        let mut apps = Vec::new();
        collect_apps(dir.path(), &mut apps, 0);
        let mut names: Vec<&str> = apps.iter().map(|app| app.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["Calculator", "TextEdit"]);
    }

    #[test]
    fn apps_in_dir_reads_desktop_entries() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("editor.desktop"),
            "[Desktop Entry]\nType=Application\nName=Editor\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("hidden.desktop"),
            "[Desktop Entry]\nType=Application\nName=Hidden\nHidden=true\n",
        )
        .unwrap();
        fs::write(dir.path().join("readme.txt"), "x").unwrap();
        let names: Vec<String> = apps_in_dir(dir.path()).into_iter().map(|app| app.name).collect();
        assert_eq!(names, ["Editor"]);
    }

    #[test]
    fn default_open_uses_the_platform_mime_handler() {
        let command = default_open_command("/tmp/photo with spaces.png").unwrap();
        let args: Vec<_> = command.get_args().collect();
        if IS_MACOS {
            assert_eq!(command.get_program(), "open");
            assert_eq!(args, [std::ffi::OsStr::new("/tmp/photo with spaces.png")]);
        } else if cfg!(target_os = "linux") {
            assert_eq!(command.get_program(), "gio");
            assert_eq!(
                args,
                [
                    std::ffi::OsStr::new("open"),
                    std::ffi::OsStr::new("/tmp/photo with spaces.png")
                ]
            );
        }
    }

    #[test]
    fn default_open_rejects_a_missing_file() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing.png");
        assert!(open_default(missing.to_str().unwrap()).is_err());
    }

    #[test]
    fn open_with_rejects_a_missing_application() {
        assert!(open_with("/tmp/file.txt", "/no/such/application").is_err());
    }

    #[test]
    fn desktop_app_parses_names_and_skips_hidden_entries() {
        let dir = tempdir().unwrap();
        let app = dir.path().join("editor.desktop");
        fs::write(
            &app,
            "[Desktop Entry]\nType=Application\nName=Text Editor\nName[de]=Texteditor\nExec=editor %f\n",
        )
        .unwrap();
        assert_eq!(desktop_app(&app).unwrap().name, "Text Editor");

        let hidden = dir.path().join("hidden.desktop");
        fs::write(
            &hidden,
            "[Desktop Entry]\nType=Application\nName=Hidden App\nNoDisplay=true\n",
        )
        .unwrap();
        assert!(desktop_app(&hidden).is_none());

        let wrong = dir.path().join("link.desktop");
        fs::write(&wrong, "[Desktop Entry]\nType=Link\nName=A Link\n").unwrap();
        assert!(desktop_app(&wrong).is_none());
    }

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

    #[cfg(target_os = "linux")]
    #[test]
    fn a_busy_program_is_spawned_once_it_is_released() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("helper");
        fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        // An open write handle makes exec fail with ETXTBSY until it closes.
        let writer = fs::OpenOptions::new().write(true).open(&path).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(40));
            drop(writer);
        });
        let mut child = spawn_helper(&mut std::process::Command::new(&path)).unwrap();
        assert!(child.wait().unwrap().success());
        release.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn wait_or_kill_waits_for_quick_programs_and_kills_stuck_ones() {
        use std::process::Command;
        use std::time::{Duration, Instant};
        let mut quick = Command::new("true").spawn().unwrap();
        assert!(wait_or_kill(&mut quick, Duration::from_secs(5)));
        let mut stuck = Command::new("sleep").arg("10").spawn().unwrap();
        let start = Instant::now();
        assert!(!wait_or_kill(&mut stuck, Duration::from_millis(100)));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "killed, not waited out"
        );
        assert!(
            stuck.try_wait().unwrap().is_some(),
            "the stuck process was reaped"
        );
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
}

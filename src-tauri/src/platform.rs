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
    wait_or_kill_unless(child, timeout, &|| false)
}

/// `wait_or_kill`, also killing `child` as soon as `stop` returns true.
pub fn wait_or_kill_unless(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
    stop: &dyn Fn() -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if std::time::Instant::now() < deadline && !stop() => {
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

/// What to change in the environment `vars` so a program from the host
/// system runs as if started outside the AppImage: each value is set anew,
/// or unset with `None`. Empty when not running from an AppImage.
///
/// The AppImage's AppRun points `LD_LIBRARY_PATH`, `PATH`, `XDG_DATA_DIRS`,
/// GTK's and GIO's variables and more into its mount, `$APPDIR`. Inherited,
/// they make the host's `gio` load the AppImage's older GLib and fail
/// ("undefined symbol"), and would hand the same libraries to the app it
/// opens. The original values aren't kept, so the mount's entries are taken
/// out of each list, and variables left empty are unset.
fn host_env_changes(
    vars: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, Option<String>)> {
    let vars: Vec<(String, String)> = vars.into_iter().collect();
    let appdir = vars
        .iter()
        .find(|(key, _)| key == "APPDIR")
        .map(|(_, value)| value.trim_end_matches('/'))
        .filter(|appdir| !appdir.is_empty());
    let Some(appdir) = appdir else {
        return Vec::new();
    };
    let inside = |entry: &str| entry == appdir || entry.starts_with(&format!("{appdir}/"));
    let mut changes = Vec::new();
    for (key, value) in &vars {
        if matches!(key.as_str(), "APPDIR" | "APPIMAGE" | "ARGV0") {
            changes.push((key.clone(), None));
            continue;
        }
        if !value.split(':').any(inside) {
            continue;
        }
        let kept: Vec<&str> = value.split(':').filter(|e| !e.is_empty() && !inside(e)).collect();
        changes.push((key.clone(), (!kept.is_empty()).then(|| kept.join(":"))));
    }
    changes
}

fn apply_env_changes(command: &mut std::process::Command, changes: Vec<(String, Option<String>)>) {
    for (key, value) in changes {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
}

/// Give `command` the host's environment rather than the AppImage's (see
/// `host_env_changes`); a no-op outside an AppImage.
pub fn host_env(command: &mut std::process::Command) {
    let vars = std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)));
    apply_env_changes(command, host_env_changes(vars));
}

/// Start an external helper (qlmanage, mdfind, gio, xdg-open) with the
/// host's environment. A spawn that fails with "text file busy" (ETXTBSY) is
/// retried briefly: it happens when another thread forks while the program
/// file is still open for writing, which the tests' stand-in scripts hit
/// when the suite runs in parallel.
pub fn spawn_helper(command: &mut std::process::Command) -> std::io::Result<std::process::Child> {
    host_env(command);
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

/// How running a launcher (`open`, `gio open`, …) went.
#[derive(Debug, PartialEq)]
enum Launch {
    Opened,
    /// The launcher program isn't installed.
    Missing,
    Failed(String),
}

/// Run a launcher and wait for it. `open`, `gio open`, `gio launch` and
/// `gtk-launch` exit as soon as the app has been asked to open the file, so
/// waiting is short, reaps the process and turns a failure (say, no app for
/// this type) into its own message.
fn launch(command: &mut std::process::Command) -> Launch {
    use std::process::Stdio;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let child = match spawn_helper(command) {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Launch::Missing,
        Err(error) => return Launch::Failed(error.to_string()),
    };
    match child.wait_with_output() {
        Ok(output) if output.status.success() => Launch::Opened,
        Ok(output) => Launch::Failed(launcher_message(&output.stderr, output.status)),
        Err(error) => Launch::Failed(error.to_string()),
    }
}

/// A launcher's error output, without the `gio: file:///…: ` prefix gio puts
/// before the reason.
fn launcher_message(stderr: &[u8], status: std::process::ExitStatus) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text.lines().rev().find(|line| !line.trim().is_empty());
    let Some(line) = line.map(str::trim) else {
        return format!("The launcher failed ({status}).");
    };
    line.strip_prefix("gio: ")
        .and_then(|rest| rest.split_once(": "))
        .map_or(line, |(_, reason)| reason)
        .to_string()
}

/// Start a launcher that may not exit until the app does (`xdg-open` runs
/// some apps in the foreground) and reap it on a thread of its own.
fn spawn_detached(command: &mut std::process::Command) -> Result<(), String> {
    let mut child = spawn_helper(command).map_err(|e| e.to_string())?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Launch `path` with the default application for its type.
pub fn open_default(path: &str) -> Result<(), String> {
    fs::metadata(path).map_err(|e| e.to_string())?;
    let opened = match launch(&mut default_open_command(path)?) {
        Launch::Opened => Ok(()),
        Launch::Failed(message) => Err(message),
        // Not every Linux desktop installs GLib's `gio` tool.
        Launch::Missing if !IS_MACOS => {
            spawn_detached(std::process::Command::new("xdg-open").arg(path))
        }
        Launch::Missing => Err("The open command is missing.".into()),
    };
    opened.map_err(|e| format!("Could not open with the default application: {e}"))
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
    let mut command = std::process::Command::new("open");
    command.arg("-a").arg(app_path).arg(path);
    launched(launch(&mut command))
}

fn linux_open_with(path: &str, app: &str) -> Result<(), String> {
    let desktop = Path::new(app);
    if !desktop.is_file() {
        return Err("That application is no longer available.".into());
    }
    let mut gio = std::process::Command::new("gio");
    gio.arg("launch").arg(desktop).arg(path);
    // `gio launch` is newer than `gtk-launch` (GLib 2.68): try the other one
    // when it is missing or fails.
    if launch(&mut gio) == Launch::Opened {
        return Ok(());
    }
    let id = desktop
        .file_name()
        .map(|name| name.to_string_lossy().trim_end_matches(".desktop").to_string())
        .unwrap_or_default();
    let mut gtk = std::process::Command::new("gtk-launch");
    gtk.arg(id).arg(path);
    launched(launch(&mut gtk))
}

fn launched(result: Launch) -> Result<(), String> {
    match result {
        Launch::Opened => Ok(()),
        Launch::Missing => Err("Could not open with that application: the launcher is missing.".into()),
        Launch::Failed(message) => Err(format!("Could not open with that application: {message}")),
    }
}

/// The window buttons the header bars draw, on each side. Empty on macOS,
/// where the system draws the traffic lights over the window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowButtons {
    pub start: Vec<String>,
    pub end: Vec<String>,
}

/// GNOME's default `button-layout`: only a close button, on the right.
const DEFAULT_BUTTON_LAYOUT: &str = "appmenu:close";

/// Parse a GNOME `button-layout` such as `'close,minimize:'` or
/// `appmenu:minimize,maximize,close`: buttons before the colon go at the
/// start, the rest at the end. Entries other than the three window buttons
/// (`appmenu`, `icon`, `spacer`) are dropped.
pub fn parse_button_layout(layout: &str) -> WindowButtons {
    let layout = layout.trim().trim_matches('\'');
    let (start, end) = layout.split_once(':').unwrap_or((layout, ""));
    let buttons = |side: &str| {
        side.split(',')
            .map(str::trim)
            .filter(|name| matches!(*name, "minimize" | "maximize" | "close"))
            .map(String::from)
            .collect()
    };
    WindowButtons {
        start: buttons(start),
        end: buttons(end),
    }
}

/// The window buttons the user chose in GNOME, or GNOME's default when the
/// setting can't be read (no `gsettings`, another desktop).
pub fn window_buttons() -> WindowButtons {
    if IS_MACOS {
        return WindowButtons {
            start: Vec::new(),
            end: Vec::new(),
        };
    }
    let mut gsettings = std::process::Command::new("gsettings");
    host_env(&mut gsettings);
    let output = gsettings
        .args(["get", "org.gnome.desktop.wm.preferences", "button-layout"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let layout = match &output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout).into_owned(),
        _ => DEFAULT_BUTTON_LAYOUT.to_string(),
    };
    parse_button_layout(&layout)
}

/// What the header bar's window buttons need from a window; `lib.rs`
/// implements it for Tauri's window.
pub trait WindowOps {
    fn minimize(&self) -> Result<(), String>;
    fn maximize(&self) -> Result<(), String>;
    fn unmaximize(&self) -> Result<(), String>;
    fn close(&self) -> Result<(), String>;
    fn is_maximized(&self) -> Result<bool, String>;
}

/// Run a window button: `minimize`, `maximize` (which toggles) or `close`;
/// `state` changes nothing. Answers whether the window is now maximized, so
/// the button can show Restore.
pub fn window_control(window: &impl WindowOps, action: &str) -> Result<bool, String> {
    match action {
        "minimize" => window.minimize()?,
        "maximize" if window.is_maximized()? => window.unmaximize()?,
        "maximize" => window.maximize()?,
        "close" => window.close()?,
        "state" => {}
        _ => return Err(format!("Unknown window action “{action}”")),
    }
    window.is_maximized()
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

// ---- Linux program installation -----------------------------------------

#[derive(Debug, Serialize)]
pub struct InstalledAppImage {
    pub name: String,
    pub path: PathBuf,
    pub desktop: PathBuf,
}

const INSTALL_EXISTS: &str = "An application with that name is already installed. Choose a different name.";

/// A real local file, never a remote path, symlink or history object. Return
/// the open file so validation and copying read the same inode.
fn program_file(path: &Path, extensions: &[&str]) -> Result<(fs::File, PathBuf), String> {
    use std::os::unix::fs::MetadataExt;
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase();
    if !path.is_absolute() || !extensions.contains(&extension.as_str()) {
        return Err("Choose a local program file of the supported type.".into());
    }
    let original = fs::symlink_metadata(path).map_err(|_| "This file is no longer available.")?;
    if !original.is_file() {
        return Err("Choose a regular file, not a folder or symbolic link.".into());
    }
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    let unsafe_path = canonical.components().any(|c| c.as_os_str() == ".versions")
        || canonical.to_string_lossy().chars().any(char::is_control);
    if unsafe_path { return Err("This location cannot be used to install or run a program.".into()); }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if original.dev() != opened.dev() || original.ino() != opened.ino() {
        return Err("The file changed. Try again.".into());
    }
    Ok((file, canonical))
}

fn application_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || matches!(name, "." | "..") { return Err("Enter an application name.".into()); }
    if name.chars().any(|c| c.is_control() || c == '/' || c == '\\') {
        return Err("Application names cannot contain separators or control characters.".into());
    }
    if name.chars().count() > 100 { return Err("Application name is too long.".into()); }
    Ok(name)
}

/// Desktop-entry escaping happens after Exec argument quoting. Percent signs
/// must be doubled so a path cannot introduce %f/%U or other field codes.
fn desktop_exec(path: &Path) -> Result<String, String> {
    let path = path.to_str().ok_or("The installation path must be valid UTF-8.")?;
    if path.chars().any(char::is_control) { return Err("The installation path contains control characters.".into()); }
    let mut argument = String::from("\"");
    for c in path.chars() {
        match c {
            '\\' | '"' | '$' | '`' => { argument.push('\\'); argument.push(c); }
            '%' => argument.push_str("%%"),
            c => argument.push(c),
        }
    }
    argument.push('"');
    Ok(argument.replace('\\', "\\\\"))
}

fn new_program_file(path: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(|e| e.to_string())
}

/// Install without executing or modifying the original. The caller supplies
/// XDG_DATA_HOME; every destination is new and a clash never overwrites.
pub fn install_appimage_at(source: &Path, name: &str, data: &Path) -> Result<InstalledAppImage, String> {
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::unix::fs::PermissionsExt;
    let name = application_name(name)?.to_string();
    let (mut input, _) = program_file(source, &["appimage"])?;
    let before = input.metadata().map_err(|e| e.to_string())?;
    let mut header = [0u8; 16];
    input.read_exact(&mut header).map_err(|_| "This isn't a supported AppImage.")?;
    if &header[..4] != b"\x7fELF" || &header[8..10] != b"AI" || ![1, 2].contains(&header[10]) {
        return Err("This isn't a supported AppImage.".into());
    }
    input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    if !data.is_absolute() || data.components().any(|c| c.as_os_str() == ".versions") {
        return Err("The application data folder is not a safe absolute path.".into());
    }
    let id = format!("io.github.ahmadsayed.filebrowser.appimage.{}", &blake3::hash(name.as_bytes()).to_hex()[..16]);
    let root = data.join("filebrowser/appimages");
    let folder = root.join(&id);
    let path = folder.join("application.AppImage");
    let applications = data.join("applications");
    let desktop = applications.join(format!("{id}.desktop"));
    let exec = desktop_exec(&path)?;
    let text = format!("[Desktop Entry]\nType=Application\nVersion=1.0\nName={name}\nExec={exec}\nIcon=application-x-executable\nTerminal=false\nStartupNotify=true\nCategories=Utility;\n");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    fs::create_dir_all(&applications).map_err(|e| e.to_string())?;
    fs::create_dir(&folder).map_err(|e| if e.kind() == std::io::ErrorKind::AlreadyExists { INSTALL_EXISTS.into() } else { e.to_string() })?;
    let staged = folder.join("launcher.desktop");
    let result = (|| {
        let mut output = new_program_file(&path)?;
        let copied = std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
        let after = input.metadata().map_err(|e| e.to_string())?;
        if copied != before.len() || before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err("The AppImage changed while installing. Try again.".into());
        }
        output.sync_all().map_err(|e| e.to_string())?;
        output.set_permissions(fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        let mut launcher = new_program_file(&staged)?;
        launcher.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        launcher.sync_all().map_err(|e| e.to_string())?;
        launcher.set_permissions(fs::Permissions::from_mode(0o644)).map_err(|e| e.to_string())?;
        // Atomic no-clobber publication; the staging file is on the same filesystem.
        fs::hard_link(&staged, &desktop).map_err(|e| if e.kind() == std::io::ErrorKind::AlreadyExists { INSTALL_EXISTS.into() } else { e.to_string() })?;
        Ok(InstalledAppImage { name, path: path.clone(), desktop })
    })();
    let _ = fs::remove_file(&staged);
    if result.is_err() {
        // Only this call's newly created staging directory, never an old install.
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&folder);
    }
    result
}

// ---- Linux shell script launching ---------------------------------------

/// Build a terminal request with all user paths passed as argv, never as
/// shell text. Bash reads the script directly, so chmod isn't necessary.
pub fn script_terminal_command(path: &Path, terminal: &Path) -> Result<std::process::Command, String> {
    use std::io::Read;
    let (file, path) = program_file(path, &["sh", "bash"])?;
    let mut start = Vec::new();
    file.take(4096).read_to_end(&mut start).map_err(|e| e.to_string())?;
    if start.contains(&0) { return Err("This file is not a text shell script.".into()); }
    let parent = path.parent().ok_or("This script has no containing folder.")?;
    let mut command = std::process::Command::new(terminal);
    command.current_dir(parent).env_remove("BASH_ENV");
    match terminal.file_name().and_then(|s| s.to_str()) {
        Some("xdg-terminal-exec") => { command.arg(format!("--dir={}", parent.display())).arg("--"); }
        Some("xterm") => { command.arg("-e"); }
        _ => { command.arg("--working-directory").arg(parent).arg("--"); }
    }
    // Fixed trusted wrapper, then the script path in $1. Keep a real terminal
    // open after exit; EOF in a headless test returns immediately.
    command.args(["/bin/bash", "--noprofile", "--norc", "-c",
        "/bin/bash -- \"$1\"; status=$?; printf '\\nScript exited with status %s.\\nPress Enter to close.\\n' \"$status\"; read -r _; exit \"$status\"",
        "filebrowser-script",
    ]).arg(path);
    host_env(&mut command);
    Ok(command)
}

/// Launch a terminal, returning only whether the request was accepted. Never
/// describe this as a successful script execution; output belongs in the terminal.
pub fn run_shell_script(path: &Path) -> Result<(), String> {
    let terminals = ["xdg-terminal-exec", "gnome-terminal", "kgx", "xterm"];
    for terminal in terminals {
        let mut command = script_terminal_command(path, Path::new(terminal))?;
        command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        if request_script_terminal(&mut command)? { return Ok(()); }
    }
    Err("No supported terminal was found. Install GNOME Terminal, GNOME Console or xterm.".into())
}

fn request_script_terminal(command: &mut std::process::Command) -> Result<bool, String> {
    match spawn_helper(command) {
        Ok(child) => { check_terminal_started(child)?; Ok(true) }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("Could not open a terminal: {e}")),
    }
}

fn check_terminal_started(mut child: std::process::Child) -> Result<(), String> {
    // Catch a launcher's immediate failure (e.g. no session bus).
    std::thread::sleep(std::time::Duration::from_millis(100));
    if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
        return if status.success() { Ok(()) } else {
            Err("The terminal could not be opened. Check your desktop session.".into())
        };
    }
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Optional desktop MIME cache refresh; GNOME also observes new launcher files.
pub fn refresh_application_cache(data: &Path) {
    let mut command = std::process::Command::new("update-desktop-database");
    command.arg(data.join("applications")).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Ok(mut child) = spawn_helper(&mut command) {
        wait_or_kill(&mut child, std::time::Duration::from_secs(5));
    }
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
    fn installing_an_appimage_registers_a_launcher_without_changing_the_download() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempdir().unwrap();
        let source = tmp.path().join("Demo.AppImage");
        let mut bytes = vec![0u8; 128];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[8..11].copy_from_slice(b"AI\x02");
        fs::write(&source, &bytes).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
        let installed = install_appimage_at(&source, "Demo", &tmp.path().join("data")).unwrap();
        assert_eq!(installed.name, "Demo");
        assert_eq!(fs::read(&source).unwrap(), bytes);
        assert_eq!(fs::metadata(&source).unwrap().permissions().mode() & 0o777, 0o640);
        assert_eq!(fs::read(&installed.path).unwrap(), bytes);
        assert_eq!(fs::metadata(&installed.path).unwrap().permissions().mode() & 0o777, 0o755);
        let desktop = fs::read_to_string(&installed.desktop).unwrap();
        assert!(desktop.contains("Name=Demo\n"));
        assert!(desktop.contains("Icon=application-x-executable\n"));
        assert!(desktop.contains("Terminal=false\n"));
    }

    #[test]
    fn a_script_runs_in_its_folder_without_chmod_or_path_interpolation() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempdir().unwrap();
        let folder = tmp.path().join("scripts with spaces");
        fs::create_dir(&folder).unwrap();
        let script = folder.join("a; touch INJECTED.sh");
        fs::write(&script, "printf 'done' > result.txt\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o600)).unwrap();
        let terminal = tmp.path().join("gnome-terminal");
        fs::write(&terminal, "#!/bin/sh\nwhile [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n").unwrap();
        fs::set_permissions(&terminal, fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = spawn_helper(&mut script_terminal_command(&script, &terminal).unwrap()).unwrap();
        assert!(wait_or_kill(&mut child, std::time::Duration::from_secs(5)));
        assert_eq!(fs::read_to_string(folder.join("result.txt")).unwrap(), "done");
        assert!(!folder.join("INJECTED.sh").exists());
        assert_eq!(fs::metadata(&script).unwrap().permissions().mode() & 0o777, 0o600);
    }

    fn appimage_fixture(root: &Path) -> PathBuf {
        let source = root.join("Demo.AppImage");
        let mut bytes = vec![0u8; 128];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[8..11].copy_from_slice(b"AI\x02");
        fs::write(&source, bytes).unwrap();
        source
    }

    #[test]
    fn appimage_name_clashes_do_not_replace_an_install_or_its_launcher() {
        let tmp = tempdir().unwrap();
        let source = appimage_fixture(tmp.path());
        let data = tmp.path().join("data");
        let first = install_appimage_at(&source, "Demo", &data).unwrap();
        fs::write(&first.path, b"edited installed application").unwrap();
        let desktop = fs::read(&first.desktop).unwrap();
        assert_eq!(install_appimage_at(&source, "Demo", &data).unwrap_err(), INSTALL_EXISTS);
        assert_eq!(fs::read(&first.path).unwrap(), b"edited installed application");
        assert_eq!(fs::read(&first.desktop).unwrap(), desktop);
    }

    #[test]
    fn launcher_clashes_roll_back_only_the_new_application() {
        let tmp = tempdir().unwrap();
        let source = appimage_fixture(tmp.path());
        let data = tmp.path().join("data");
        let first = install_appimage_at(&source, "Demo", &data).unwrap();
        let folder = first.path.parent().unwrap();
        fs::remove_file(&first.path).unwrap();
        fs::remove_dir(folder).unwrap();
        fs::write(&first.desktop, b"user's launcher").unwrap();
        assert_eq!(install_appimage_at(&source, "Demo", &data).unwrap_err(), INSTALL_EXISTS);
        assert_eq!(fs::read(&first.desktop).unwrap(), b"user's launcher");
        assert!(!folder.exists());
        assert_eq!(fs::metadata(&source).unwrap().len(), 128);
    }

    #[test]
    fn appimage_installation_refuses_invalid_programs_names_and_locations() {
        let tmp = tempdir().unwrap();
        let source = appimage_fixture(tmp.path());
        let data = tmp.path().join("data");
        for name in ["", "..", "Bad/Name", "Bad\\Name", "Bad\nExec=/bin/sh"] {
            assert!(install_appimage_at(&source, name, &data).is_err());
        }
        assert!(install_appimage_at(&source, &"x".repeat(101), &data).is_err());
        let bogus = tmp.path().join("bogus.AppImage");
        fs::write(&bogus, b"not an AppImage").unwrap();
        assert!(install_appimage_at(&bogus, "Bogus", &data).is_err());
        let folder = tmp.path().join("folder.AppImage");
        fs::create_dir(&folder).unwrap();
        assert!(install_appimage_at(&folder, "Folder", &data).is_err());
        let alias = tmp.path().join("alias.AppImage");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        assert!(install_appimage_at(&alias, "Alias", &data).is_err());
        assert!(install_appimage_at(Path::new("share://r1/app.AppImage"), "Remote", &data).is_err());
        assert!(install_appimage_at(&source, "Demo", Path::new("relative" )).is_err());
        assert!(install_appimage_at(&source, "Demo", &tmp.path().join(".versions/data")).is_err());
        let versions = tmp.path().join(".versions");
        fs::create_dir(&versions).unwrap();
        let internal = versions.join("internal.AppImage");
        fs::copy(&source, &internal).unwrap();
        assert!(install_appimage_at(&internal, "History", &data).is_err());
        assert!(!data.exists(), "invalid input must not create installation directories");
    }

    #[test]
    fn appimage_launcher_quotes_special_characters_and_preserves_unicode_names() {
        let tmp = tempdir().unwrap();
        let source = appimage_fixture(tmp.path());
        let data = tmp.path().join("data with %f $HOME `echo x` \\\"quotes\"");
        let installed = install_appimage_at(&source, "Café Editor", &data).unwrap();
        let desktop = fs::read_to_string(installed.desktop).unwrap();
        assert!(desktop.contains("Name=Café Editor\n"));
        assert!(desktop.contains("%%f"), "literal percent must not become a desktop field code");
        assert!(desktop.contains("\\\\$HOME"));
        assert!(desktop.contains("\\\\`echo x\\\\`"));
        assert!(desktop.lines().find(|s| s.starts_with("Exec=")).unwrap().starts_with("Exec=\"/"));
    }

    #[test]
    fn script_requests_refuse_binary_remote_missing_and_history_files() {
        let tmp = tempdir().unwrap();
        let terminal = Path::new("gnome-terminal");
        let binary = tmp.path().join("binary.sh");
        fs::write(&binary, b"hello\0there").unwrap();
        assert!(script_terminal_command(&binary, terminal).is_err());
        for path in ["sftp://user@host/a.sh", "share://r1/a.sh", "relative.sh"] {
            assert!(script_terminal_command(Path::new(path), terminal).is_err());
        }
        assert!(script_terminal_command(&tmp.path().join("missing.sh"), terminal).is_err());
        let internal = tmp.path().join(".versions/script.sh");
        fs::create_dir(internal.parent().unwrap()).unwrap();
        fs::write(&internal, b"echo hello").unwrap();
        assert!(script_terminal_command(&internal, terminal).is_err());
    }

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

    /// A stand-in launcher: a shell script with `body`, made executable.
    #[cfg(unix)]
    fn fake_launcher(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("launcher");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn a_launcher_reports_success_its_error_or_that_it_is_missing() {
        use std::process::Command;
        let dir = tempdir().unwrap();
        let ok = fake_launcher(dir.path(), "exit 0");
        assert_eq!(launch(&mut Command::new(&ok)), Launch::Opened);

        let fail = fake_launcher(
            dir.path(),
            r#"echo "gio: file:///tmp/a.xyz: No application is registered as handling this file" >&2; exit 2"#,
        );
        assert_eq!(
            launch(&mut Command::new(&fail)),
            Launch::Failed("No application is registered as handling this file".into())
        );

        let silent = fake_launcher(dir.path(), "exit 3");
        assert!(matches!(
            launch(&mut Command::new(&silent)),
            Launch::Failed(message) if message.starts_with("The launcher failed")
        ));

        assert_eq!(
            launch(&mut Command::new(dir.path().join("no-such-launcher"))),
            Launch::Missing
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_detached_launcher_is_reaped_after_it_exits() {
        use std::process::Command;
        let dir = tempdir().unwrap();
        let marker = dir.path().join("ran");
        let script = fake_launcher(dir.path(), &format!("touch '{}'", marker.display()));
        spawn_detached(&mut Command::new(&script)).unwrap();
        for _ in 0..200 {
            if marker.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(marker.exists());
        assert!(spawn_detached(&mut Command::new(dir.path().join("missing"))).is_err());
    }

    #[test]
    fn button_layout_splits_at_the_colon_and_keeps_window_buttons() {
        let buttons = |start: &[&str], end: &[&str]| WindowButtons {
            start: start.iter().map(|s| s.to_string()).collect(),
            end: end.iter().map(|s| s.to_string()).collect(),
        };
        // gsettings prints the value quoted, with a newline.
        assert_eq!(parse_button_layout("'appmenu:close'\n"), buttons(&[], &["close"]));
        assert_eq!(
            parse_button_layout("appmenu:minimize,maximize,close"),
            buttons(&[], &["minimize", "maximize", "close"])
        );
        assert_eq!(
            parse_button_layout("close,minimize,maximize:icon"),
            buttons(&["close", "minimize", "maximize"], &[])
        );
        // No colon: everything is at the start, as in GTK.
        assert_eq!(parse_button_layout("close"), buttons(&["close"], &[]));
        assert_eq!(parse_button_layout(""), buttons(&[], &[]));
        assert_eq!(
            parse_button_layout(DEFAULT_BUTTON_LAYOUT),
            buttons(&[], &["close"])
        );
    }

    /// A window that records what it was asked to do.
    #[derive(Default)]
    struct FakeWindow {
        maximized: std::cell::Cell<bool>,
        calls: std::cell::RefCell<Vec<&'static str>>,
    }

    impl WindowOps for FakeWindow {
        fn minimize(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("minimize");
            Ok(())
        }
        fn maximize(&self) -> Result<(), String> {
            self.maximized.set(true);
            self.calls.borrow_mut().push("maximize");
            Ok(())
        }
        fn unmaximize(&self) -> Result<(), String> {
            self.maximized.set(false);
            self.calls.borrow_mut().push("unmaximize");
            Ok(())
        }
        fn close(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("close");
            Ok(())
        }
        fn is_maximized(&self) -> Result<bool, String> {
            Ok(self.maximized.get())
        }
    }

    #[test]
    fn window_control_toggles_maximize_and_reports_the_state() {
        let window = FakeWindow::default();
        assert_eq!(window_control(&window, "state"), Ok(false));
        assert_eq!(window_control(&window, "maximize"), Ok(true));
        assert_eq!(window_control(&window, "maximize"), Ok(false));
        assert_eq!(window_control(&window, "minimize"), Ok(false));
        assert_eq!(window_control(&window, "close"), Ok(false));
        assert!(window_control(&window, "fullscreen").is_err());
        assert_eq!(
            *window.calls.borrow(),
            ["maximize", "unmaximize", "minimize", "close"]
        );
    }

    #[test]
    fn window_buttons_are_left_to_macos_itself() {
        if IS_MACOS {
            assert_eq!(window_buttons(), parse_button_layout(""));
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

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn host_programs_lose_what_the_appimage_added_to_the_environment() {
        // What the AppImage's AppRun sets (from a real 0.5.1 launch), next
        // to the user's own variables.
        let m = "/tmp/.mount_file-bgdPIhE";
        let env = vars(&[
            ("APPDIR", m),
            ("APPIMAGE", "/home/me/.local/bin/file-browser.AppImage"),
            ("ARGV0", "/home/me/.local/bin/file-browser.AppImage"),
            ("LD_LIBRARY_PATH", &format!("{m}/usr/lib/:{m}/usr/lib/x86_64-linux-gnu/")),
            ("PATH", &format!("{m}/usr/bin/:{m}/bin/:/home/me/.cargo/bin:/usr/bin")),
            ("XDG_DATA_DIRS", &format!("{m}/usr/share/:{m}/usr/share:/usr/share:/usr/local/share/")),
            ("GTK_DATA_PREFIX", m),
            ("GIO_MODULE_DIR", &format!("{m}//usr/lib/gio/modules")),
            ("PYTHONPATH", &format!("{m}/usr/share/pyshared/:")),
            ("HOME", "/home/me"),
            // Another folder whose name merely starts like the mount stays.
            ("MY_TOOLS", &format!("{m}-other/bin:/opt/tools")),
        ]);
        let mut changes = host_env_changes(env);
        changes.sort();
        let set = |k: &str, v: &str| (k.to_string(), Some(v.to_string()));
        let unset = |k: &str| (k.to_string(), None);
        assert_eq!(
            changes,
            vec![
                unset("APPDIR"),
                unset("APPIMAGE"),
                unset("ARGV0"),
                unset("GIO_MODULE_DIR"),
                unset("GTK_DATA_PREFIX"),
                unset("LD_LIBRARY_PATH"),
                set("PATH", "/home/me/.cargo/bin:/usr/bin"),
                unset("PYTHONPATH"),
                set("XDG_DATA_DIRS", "/usr/share:/usr/local/share/"),
            ]
        );
    }

    #[test]
    fn outside_an_appimage_the_environment_is_left_alone() {
        let env = vars(&[("PATH", "/usr/bin"), ("LD_LIBRARY_PATH", "/opt/lib")]);
        assert!(host_env_changes(env).is_empty());
        assert!(host_env_changes(vars(&[("APPDIR", ""), ("PATH", "/usr/bin")])).is_empty());
    }

    #[test]
    fn a_command_gets_the_host_environment() {
        let mut command = std::process::Command::new("gio");
        apply_env_changes(
            &mut command,
            vec![
                ("LD_LIBRARY_PATH".to_string(), None),
                ("PATH".to_string(), Some("/usr/bin".to_string())),
            ],
        );
        let envs: Vec<_> = command.get_envs().collect();
        assert!(envs.contains(&(std::ffi::OsStr::new("LD_LIBRARY_PATH"), None)));
        assert!(envs.contains(&(std::ffi::OsStr::new("PATH"), Some(std::ffi::OsStr::new("/usr/bin")))));
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

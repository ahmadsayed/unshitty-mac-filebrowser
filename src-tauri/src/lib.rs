mod config_store;
mod fs_ops;
mod recent;
mod thumbnails;
mod trash_ops;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;

fn config_file(app: &tauri::AppHandle, name: &str) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(name))
}

/// Folders the user pinned to the sidebar ("Add to Bookmarks").
#[tauri::command]
fn load_pins(app: tauri::AppHandle) -> Vec<String> {
    config_file(&app, "pins.json")
        .map(|p| config_store::read_paths(&p))
        .unwrap_or_default()
}

#[tauri::command]
fn save_pins(app: tauri::AppHandle, paths: Vec<String>) -> Result<(), String> {
    config_store::write_json(&config_file(&app, "pins.json")?, &paths)
}

/// A persisted UI flag, e.g. whether the Full Disk Access banner was dismissed.
#[tauri::command]
fn get_flag(app: tauri::AppHandle, name: String) -> bool {
    config_file(&app, "flags.json")
        .map(|p| config_store::read_flags(&p).get(&name).copied().unwrap_or(false))
        .unwrap_or(false)
}

#[tauri::command]
fn set_flag(app: tauri::AppHandle, name: String, value: bool) -> Result<(), String> {
    let path = config_file(&app, "flags.json")?;
    let mut flags = config_store::read_flags(&path);
    flags.insert(name, value);
    config_store::write_json(&path, &flags)
}

/// Paths of starred files and folders, in the order they were starred.
#[tauri::command]
fn load_starred(app: tauri::AppHandle) -> Vec<String> {
    config_file(&app, "starred.json")
        .map(|p| config_store::read_paths(&p))
        .unwrap_or_default()
}

#[tauri::command]
fn save_starred(app: tauri::AppHandle, paths: Vec<String>) -> Result<(), String> {
    config_store::write_json(&config_file(&app, "starred.json")?, &paths)
}

/// The "Starred" listing: starred items that still exist.
#[tauri::command(async)]
fn starred_files(app: tauri::AppHandle, show_hidden: bool) -> Vec<fs_ops::Entry> {
    let paths = config_file(&app, "starred.json")
        .map(|p| config_store::read_paths(&p))
        .unwrap_or_default();
    fs_ops::entries_for(&paths, show_hidden)
}

#[tauri::command]
fn has_full_disk_access(app: tauri::AppHandle) -> bool {
    app.path()
        .home_dir()
        .map(|home| config_store::has_full_disk_access(&home))
        .unwrap_or(true)
}

/// The "Recent" listing: files opened lately, here or in other apps.
#[tauri::command(async)]
fn recent_files(app: tauri::AppHandle, show_hidden: bool) -> Vec<recent::RecentEntry> {
    let Ok(home) = app.path().home_dir() else { return Vec::new() };
    let store = config_file(&app, "recent.json")
        .map(|p| recent::read_store(&p))
        .unwrap_or_default();
    let spotlight = recent::spotlight(&home, recent::MAX_AGE_DAYS);
    recent::collect(&store, spotlight, &home, show_hidden, recent::now())
}

/// Note that the user opened `path` from this app.
#[tauri::command]
fn add_recent(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let file = config_file(&app, "recent.json")?;
    let mut store = recent::read_store(&file);
    recent::record_open(&mut store, &path, recent::now());
    config_store::write_json(&file, &store)
}

/// "Remove from Recent".
#[tauri::command]
fn forget_recent(app: tauri::AppHandle, paths: Vec<String>) -> Result<(), String> {
    let file = config_file(&app, "recent.json")?;
    let mut store = recent::read_store(&file);
    recent::forget(&mut store, &paths, recent::now());
    config_store::write_json(&file, &store)
}

/// Serializes read-modify-write of `trash_log.json`.
static TRASH_LOG: Mutex<()> = Mutex::new(());

fn home_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path().home_dir().map_err(|e| e.to_string())
}

/// Move `path` to the Trash. Returns where it went, when that's known, so
/// the move can be undone.
#[tauri::command(async)]
fn trash_entry(app: tauri::AppHandle, path: String) -> Result<Option<String>, String> {
    let home = home_dir(&app)?;
    let origin = PathBuf::from(&path);
    let trashed = trash_ops::trash_and_locate(&origin, &home)?;
    if let (Some(trashed), Ok(log_path)) = (&trashed, config_file(&app, "trash_log.json")) {
        let _guard = TRASH_LOG.lock().unwrap_or_else(|e| e.into_inner());
        let mut log = trash_ops::read_log(&log_path);
        trash_ops::record(&mut log, trashed, &origin, recent::now());
        let _ = config_store::write_json(&log_path, &log);
    }
    Ok(trashed.map(|p| p.to_string_lossy().to_string()))
}

/// Finder's "Put Back" records for the home Trash.
fn put_back(home: &Path) -> std::collections::HashMap<String, PathBuf> {
    trash_ops::read_put_back(&home.join(".Trash/.DS_Store"), Path::new("/"))
}

/// Original location and trash date of items in the Trash.
#[tauri::command(async)]
fn trash_info(app: tauri::AppHandle, paths: Vec<String>) -> Vec<trash_ops::TrashInfo> {
    let log = config_file(&app, "trash_log.json")
        .map(|p| trash_ops::read_log(&p))
        .unwrap_or_default();
    let records = home_dir(&app).map(|h| put_back(&h)).unwrap_or_default();
    trash_ops::info(&paths, &log, &records)
}

/// "Restore From Trash" and Undo: move an item back where it came from.
/// Returns the restored path.
#[tauri::command(async)]
fn restore_entry(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let log_path = config_file(&app, "trash_log.json")?;
    let _guard = TRASH_LOG.lock().unwrap_or_else(|e| e.into_inner());
    let mut log = trash_ops::read_log(&log_path);
    let trashed = PathBuf::from(&path);
    let origin = trash_ops::origin_of(&trashed, &log, &put_back(&home_dir(&app)?))
        .ok_or_else(|| "The original location is unknown".to_string())?;
    trash_ops::restore(&trashed, &origin)?;
    log.items.remove(&path);
    let _ = config_store::write_json(&log_path, &log);
    Ok(origin.to_string_lossy().to_string())
}

/// Delete an item in the Trash for good.
#[tauri::command(async)]
fn delete_permanently(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    let trash_dir = trash_ops::trash_dir_for(&path, &home_dir(&app)?);
    trash_ops::delete_permanently(&path, &trash_dir)
}

/// Delete everything in the home Trash.
#[tauri::command(async)]
fn empty_trash(app: tauri::AppHandle) -> Result<(), String> {
    let home = home_dir(&app)?;
    trash_ops::empty(&home.join(".Trash"))
}

fn thumbnail_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|d| d.join("thumbnails"))
}

/// PNG bytes of a thumbnail for `path`, at most `size` pixels square; empty
/// when the file has no thumbnail. Raw bytes avoid JSON-encoding the image.
#[tauri::command(async)]
fn thumbnail(app: tauri::AppHandle, path: String, size: u32) -> tauri::ipc::Response {
    let png = thumbnail_dir(&app)
        .and_then(|dir| thumbnails::thumbnail(Path::new(&path), size.clamp(32, 1024), &dir).ok().flatten())
        .and_then(|p| std::fs::read(p).ok())
        .unwrap_or_default();
    tauri::ipc::Response::new(png)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Drop thumbnails nobody has needed for two months.
            if let Some(dir) = thumbnail_dir(app.handle()) {
                std::thread::spawn(move || thumbnails::prune(&dir, std::time::Duration::from_secs(60 * 86_400)));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            fs_ops::list_dir,
            fs_ops::item_counts,
            fs_ops::make_dir,
            fs_ops::rename_entry,
            fs_ops::path_exists,
            fs_ops::path_info,
            fs_ops::copy_entry,
            fs_ops::move_entry,
            trash_entry,
            trash_info,
            restore_entry,
            delete_permanently,
            empty_trash,
            load_pins,
            save_pins,
            get_flag,
            set_flag,
            has_full_disk_access,
            recent_files,
            load_starred,
            save_starred,
            starred_files,
            add_recent,
            forget_recent,
            thumbnail,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

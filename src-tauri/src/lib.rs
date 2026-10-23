mod config_store;
mod fs_ops;
mod thumbnails;

use std::path::{Path, PathBuf};
use tauri::Manager;

fn config_file(app: &tauri::AppHandle, name: &str) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(name))
}

/// Folders the user pinned to the sidebar ("Add to Bookmarks").
#[tauri::command]
fn load_pins(app: tauri::AppHandle) -> Vec<String> {
    config_file(&app, "pins.json")
        .map(|p| config_store::read_pins(&p))
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

#[tauri::command]
fn has_full_disk_access(app: tauri::AppHandle) -> bool {
    app.path()
        .home_dir()
        .map(|home| config_store::has_full_disk_access(&home))
        .unwrap_or(true)
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
            fs_ops::trash_entry,
            load_pins,
            save_pins,
            get_flag,
            set_flag,
            has_full_disk_access,
            thumbnail,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

mod cloud;
mod config_store;
mod fs_ops;
mod platform;
mod properties;
mod recent;
mod thumbnails;
mod trash_ops;
mod versions;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;

use cloud::CloudState;
use versions::history::History;
use versions::state::{Transfer, VersionsState};
use versions::{VersionDiff, VersionInfo, restore};

fn config_file(app: &tauri::AppHandle, name: &str) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(name))
}

fn lock_state<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, String> {
    mutex
        .lock()
        .map_err(|_| "Google Drive state is unavailable.".to_string())
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
        .map(|p| {
            config_store::read_flags(&p)
                .get(&name)
                .copied()
                .unwrap_or(false)
        })
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
#[tauri::command]
async fn starred_files(
    app: tauri::AppHandle,
    show_hidden: bool,
) -> Result<Vec<fs_ops::Entry>, String> {
    blocking(move || {
        let paths = config_file(&app, "starred.json")
            .map(|p| config_store::read_paths(&p))
            .unwrap_or_default();
        fs_ops::entries_for(&paths, show_hidden)
    })
    .await
}

#[tauri::command]
fn has_full_disk_access(app: tauri::AppHandle) -> bool {
    app.path()
        .home_dir()
        .map(|home| config_store::has_full_disk_access(&home))
        .unwrap_or(true)
}

/// The "Recent" listing: files opened lately, here or in other apps.
#[tauri::command]
async fn recent_files(
    app: tauri::AppHandle,
    show_hidden: bool,
) -> Result<Vec<recent::RecentEntry>, String> {
    blocking(move || {
        let Ok(home) = app.path().home_dir() else {
            return Vec::new();
        };
        let store = config_file(&app, "recent.json")
            .map(|p| recent::read_store(&p))
            .unwrap_or_default();
        let system = recent::system_recent(&home, recent::MAX_AGE_DAYS);
        recent::collect(&store, system, &home, show_hidden, recent::now())
    })
    .await
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

// ---------------------------------------------------------------------------
// Slow commands
//
// `#[tauri::command(async)]` on a plain function runs it on the async
// runtime's worker threads, one per CPU core, which every async command
// shares. A big copy, a slow disk or thumbnails waiting their turn would hold
// those threads and stall unrelated commands such as opening a folder. So
// slow commands are `async` and hand their work to Tauri's blocking-thread
// pool, which grows as needed.
// ---------------------------------------------------------------------------

/// Run `work` on the blocking-thread pool and wait for it without holding an
/// async worker.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("Background task failed: {}", e))
}

#[tauri::command]
async fn list_dir(
    app: tauri::AppHandle,
    path: String,
    show_hidden: bool,
) -> Result<Vec<fs_ops::Entry>, String> {
    if cloud::is_cloud(&path) {
        return blocking(move || {
            let state = app.state::<CloudState>();
            cloud::service::list_dir(&state, &path, show_hidden)
        })
        .await?;
    }
    blocking(move || fs_ops::list_dir(path, show_hidden)).await?
}

#[tauri::command]
async fn item_counts(
    paths: Vec<String>,
    show_hidden: bool,
) -> Result<Vec<Option<fs_ops::FolderSummary>>, String> {
    blocking(move || {
        let mut out: Vec<Option<fs_ops::FolderSummary>> = (0..paths.len()).map(|_| None).collect();
        let mut local = Vec::new();
        let mut positions = Vec::new();
        for (index, path) in paths.iter().enumerate() {
            if !cloud::is_cloud(path) {
                local.push(path.clone());
                positions.push(index);
            }
        }
        for (position, summary) in positions
            .into_iter()
            .zip(fs_ops::item_counts(local, show_hidden))
        {
            out[position] = summary;
        }
        Ok(out)
    })
    .await?
}

#[tauri::command]
async fn path_info(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<Vec<Option<fs_ops::PathInfo>>, String> {
    blocking(move || {
        let state = app.state::<CloudState>();
        let mut out = Vec::with_capacity(paths.len());
        for path in &paths {
            if cloud::is_cloud(path) {
                out.push(cloud::service::path_info(&state, path).ok());
            } else {
                out.push(
                    fs_ops::path_info(vec![path.clone()])
                        .into_iter()
                        .next()
                        .flatten(),
                );
            }
        }
        Ok(out)
    })
    .await?
}

/// Size, dates, permissions and contents for the Properties dialog.
#[tauri::command]
async fn file_properties(
    app: tauri::AppHandle,
    path: String,
    show_hidden: bool,
) -> Result<properties::FileProperties, String> {
    if cloud::is_cloud(&path) {
        return blocking(move || {
            let state = app.state::<CloudState>();
            cloud::service::file_properties(&state, &path, show_hidden)
        })
        .await?;
    }
    blocking(move || properties::file_properties(&path, show_hidden)).await?
}

/// Recursive size of a folder, shown after the Properties dialog is open.
/// `walk` lets `cancel_folder_size` stop it when the dialog closes.
#[tauri::command]
async fn folder_size(path: String, walk: u64) -> Result<u64, String> {
    if cloud::is_cloud(&path) {
        return Err(cloud::service::unsupported());
    }
    blocking(move || properties::folder_size(&path, || properties::walk_cancelled(walk))).await?
}

/// A fresh id for `folder_size`.
#[tauri::command]
fn new_folder_walk() -> u64 {
    properties::new_walk_id()
}

/// Stop folder-size walk `walk` and every earlier one.
#[tauri::command]
fn cancel_folder_size(walk: u64) {
    properties::cancel_walks_up_to(walk);
}

#[tauri::command]
fn make_dir(path: String) -> Result<(), String> {
    if cloud::is_cloud(&path) {
        return Err(cloud::service::unsupported());
    }
    fs_ops::make_dir(path)
}

#[tauri::command]
async fn copy_entry(
    app: tauri::AppHandle,
    from: String,
    to: String,
    overwrite: bool,
) -> Result<(), String> {
    if cloud::is_cloud(&from) || cloud::is_cloud(&to) {
        return Err(cloud::service::unsupported());
    }
    blocking(move || {
        let state = app.state::<VersionsState>();
        let (from_path, to_path) = (PathBuf::from(&from), PathBuf::from(&to));
        state.transfer(&from_path, &to_path, Transfer::Copy, || {
            fs_ops::copy_entry(from, to, overwrite)
        })
    })
    .await?
}

#[tauri::command]
async fn move_entry(
    app: tauri::AppHandle,
    from: String,
    to: String,
    overwrite: bool,
) -> Result<(), String> {
    if cloud::is_cloud(&from) || cloud::is_cloud(&to) {
        return Err(cloud::service::unsupported());
    }
    blocking(move || {
        let state = app.state::<VersionsState>();
        let (from_path, to_path) = (PathBuf::from(&from), PathBuf::from(&to));
        state.transfer(&from_path, &to_path, Transfer::Move, || {
            fs_ops::move_entry(from, to, overwrite)
        })
    })
    .await?
}

#[tauri::command]
async fn rename_entry(app: tauri::AppHandle, from: String, to: String) -> Result<(), String> {
    if cloud::is_cloud(&from) || cloud::is_cloud(&to) {
        return Err(cloud::service::unsupported());
    }
    blocking(move || {
        let state = app.state::<VersionsState>();
        let (from_path, to_path) = (PathBuf::from(&from), PathBuf::from(&to));
        state.transfer(&from_path, &to_path, Transfer::Move, || {
            fs_ops::rename_entry(from, to)
        })
    })
    .await?
}

/// Serializes read-modify-write of `trash_log.json`.
static TRASH_LOG: Mutex<()> = Mutex::new(());

fn home_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path().home_dir().map_err(|e| e.to_string())
}

/// Move `path` to the Trash. Returns where it went, when that's known, so
/// the move can be undone.
#[tauri::command]
async fn trash_entry(app: tauri::AppHandle, path: String) -> Result<Option<String>, String> {
    if cloud::is_cloud(&path) {
        return Err(cloud::service::unsupported());
    }
    blocking(move || {
        let home = home_dir(&app)?;
        let origin = PathBuf::from(&path);
        let state = app.state::<VersionsState>();
        let trashed = state.trash(&origin, || trash_ops::trash_and_locate(&origin, &home))?;
        if let (Some(trashed), Ok(log_path)) = (&trashed, config_file(&app, "trash_log.json")) {
            let _guard = TRASH_LOG.lock().unwrap_or_else(|e| e.into_inner());
            let mut log = trash_ops::read_log(&log_path);
            trash_ops::record(&mut log, trashed, &origin, recent::now());
            let _ = config_store::write_json(&log_path, &log);
        }
        Ok(trashed.map(|p| p.to_string_lossy().to_string()))
    })
    .await?
}

/// Finder's "Put Back" records for the home Trash. macOS keeps them in the
/// Trash's `.DS_Store`; other platforms have no such file.
fn put_back(home: &Path) -> std::collections::HashMap<String, PathBuf> {
    if cfg!(target_os = "macos") {
        return trash_ops::read_put_back(&home.join(".Trash/.DS_Store"), Path::new("/"));
    }
    std::collections::HashMap::new()
}

/// Original location and trash date of items in the Trash.
#[tauri::command]
async fn trash_info(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<Vec<trash_ops::TrashInfo>, String> {
    blocking(move || {
        let log = config_file(&app, "trash_log.json")
            .map(|p| trash_ops::read_log(&p))
            .unwrap_or_default();
        let Ok(home) = home_dir(&app) else {
            return Vec::new();
        };
        trash_ops::info(&paths, &home, &log, &put_back(&home))
    })
    .await
}

/// "Restore From Trash" and Undo: move an item back where it came from.
/// Returns the restored path.
#[tauri::command]
async fn restore_entry(app: tauri::AppHandle, path: String) -> Result<String, String> {
    if cloud::is_cloud(&path) {
        return Err(cloud::service::unsupported());
    }
    blocking(move || {
        let log_path = config_file(&app, "trash_log.json")?;
        let _guard = TRASH_LOG.lock().unwrap_or_else(|e| e.into_inner());
        let mut log = trash_ops::read_log(&log_path);
        let home = home_dir(&app)?;
        let trashed = PathBuf::from(&path);
        let trash_dir = trash_ops::trash_dir_for(&trashed, &home);
        let origin = trash_ops::origin_of(&trashed, &trash_dir, &log, &put_back(&home))
            .ok_or_else(|| "The original location is unknown".to_string())?;
        trash_ops::restore(&trashed, &origin, &trash_dir)?;
        log.items.remove(&path);
        let _ = config_store::write_json(&log_path, &log);
        Ok(origin.to_string_lossy().to_string())
    })
    .await?
}

/// Delete an item in the Trash for good.
#[tauri::command]
async fn delete_permanently(app: tauri::AppHandle, path: String) -> Result<(), String> {
    if cloud::is_cloud(&path) {
        return Err(cloud::service::unsupported());
    }
    blocking(move || {
        let path = PathBuf::from(path);
        let trash_dir = trash_ops::trash_dir_for(&path, &home_dir(&app)?);
        trash_ops::delete_permanently(&path, &trash_dir)
    })
    .await?
}

/// The folder the Trash view lists. On Linux this is
/// `~/.local/share/Trash/files`, not `~/.Trash`, and is created if missing.
#[tauri::command]
fn trash_dir(app: tauri::AppHandle) -> String {
    home_dir(&app)
        .map(|home| {
            trash_ops::ensure_items_root(&home)
                .to_string_lossy()
                .to_string()
        })
        .unwrap_or_default()
}

/// Delete everything in the home Trash.
/// True when `path` is inside the user's Trash; versioned_root ignores it.
fn in_trash(app: &tauri::AppHandle, path: &Path) -> bool {
    let Ok(home) = app.path().home_dir() else {
        return false;
    };
    path.starts_with(trash_ops::home_trash(&home))
        || path.starts_with(trash_ops::ensure_items_root(&home))
}

/// New Folder with "Keep versions" ticked. The folder is kept even if the
/// history cannot be created (D13).
#[tauri::command]
async fn make_versioned_dir(app: tauri::AppHandle, path: String) -> Result<(), String> {
    blocking(move || {
        let root = PathBuf::from(&path);
        fs_ops::make_dir(path)?;
        let state = app.state::<VersionsState>();
        match History::create(&root) {
            Ok(history) => {
                state.adopt(history);
                Ok(())
            }
            Err(error) => Err(format!(
                "The folder was created, but versions could not be turned on: {error}"
            )),
        }
    })
    .await?
}

/// The nearest versioned root for a location, if any. Cheap: it reads
/// `.versions/meta.json`, not SQLite.
#[tauri::command]
async fn versioned_root(app: tauri::AppHandle, path: String) -> Option<String> {
    if cloud::is_cloud(&path) {
        return None;
    }
    blocking(move || {
        let path = PathBuf::from(&path);
        if in_trash(&app, &path) || versions::is_internal_path(&path) {
            return None;
        }
        let state = app.state::<VersionsState>();
        let root = state.root_quick(&path)?;
        if !state.is_open(&root) {
            // Opening checks the index against meta.json and starts the
            // watcher; navigation shouldn't wait for it (spec: background).
            let handle = app.clone();
            let opening = root.clone();
            std::thread::spawn(move || {
                if let Err(e) = handle.state::<VersionsState>().open_registered(&opening) {
                    eprintln!("versions: could not open {}: {e}", opening.display());
                }
            });
        }
        Some(root.to_string_lossy().to_string())
    })
    .await
    .ok()
    .flatten()
}

fn history_and_rel(
    app: &tauri::AppHandle,
    path: &str,
) -> Result<(std::sync::Arc<History>, PathBuf), String> {
    let state = app.state::<VersionsState>();
    if versions::is_internal_path(Path::new(path)) {
        return Err("This folder does not keep versions.".to_string());
    }
    let Some((history, rel)) = state
        .history_for(Path::new(path))
        .map_err(|e| e.to_string())?
    else {
        return Err("This folder does not keep versions.".to_string());
    };
    Ok((history, rel))
}

#[tauri::command]
async fn list_versions(
    app: tauri::AppHandle,
    path: String,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<VersionInfo>, String> {
    blocking(move || {
        let (history, rel) = history_and_rel(&app, &path)?;
        let rel = versions::path_bytes(&rel);
        restore::list_infos(&history, &rel, limit, offset).map_err(|e| e.to_string())
    })
    .await?
}

#[tauri::command]
async fn version_count(app: tauri::AppHandle, path: String) -> Result<u64, String> {
    blocking(move || {
        let (history, rel) = history_and_rel(&app, &path)?;
        let rel = versions::path_bytes(&rel);
        restore::version_count(&history, &rel).map_err(|e| e.to_string())
    })
    .await?
}

#[tauri::command]
async fn version_diff(
    app: tauri::AppHandle,
    path: String,
    version: i64,
) -> Result<VersionDiff, String> {
    blocking(move || {
        let (history, rel) = history_and_rel(&app, &path)?;
        let rel = versions::path_bytes(&rel);
        restore::diff(&history, &rel, version).map_err(|e| e.to_string())
    })
    .await?
}

#[tauri::command]
async fn restore_version_copy(
    app: tauri::AppHandle,
    path: String,
    version: i64,
    name: String,
) -> Result<String, String> {
    blocking(move || {
        let (history, rel) = history_and_rel(&app, &path)?;
        let rel = versions::path_bytes(&rel);
        restore::restore_copy(&history, &rel, version, &name)
            .map(|path| path.to_string_lossy().to_string())
            .map_err(|e| e.to_string())
    })
    .await?
}

/// Materialize a version as a read-only cache copy for the default app.
#[tauri::command]
async fn open_version(
    app: tauri::AppHandle,
    path: String,
    version: i64,
    name: String,
) -> Result<String, String> {
    blocking(move || {
        let (history, rel) = history_and_rel(&app, &path)?;
        let rel = versions::path_bytes(&rel);
        let base = opened_versions_dir(&app)?;
        restore::open_copy(&history, &rel, version, &name, &base)
            .map(|path| path.to_string_lossy().to_string())
            .map_err(|e| e.to_string())
    })
    .await?
}

#[tauri::command]
async fn replace_with_version(
    app: tauri::AppHandle,
    path: String,
    version: i64,
) -> Result<(), String> {
    blocking(move || {
        let (history, rel) = history_and_rel(&app, &path)?;
        let rel = versions::path_bytes(&rel);
        restore::replace_with_version(&history, &rel, version).map_err(|e| e.to_string())
    })
    .await?
}

#[tauri::command]
async fn deleted_files(
    app: tauri::AppHandle,
    dir: String,
) -> Result<Vec<versions::DeletedFile>, String> {
    blocking(move || {
        let (history, rel) = history_and_rel(&app, &dir)?;
        let rel = versions::path_bytes(&rel);
        let filter = if rel.is_empty() {
            None
        } else {
            Some(rel.as_slice())
        };
        restore::deleted_files(&history, filter).map_err(|e| e.to_string())
    })
    .await?
}

#[tauri::command]
async fn restore_deleted(
    app: tauri::AppHandle,
    path: String,
    fallback_name: String,
) -> Result<String, String> {
    blocking(move || {
        let (history, rel) = history_and_rel(&app, &path)?;
        let rel = versions::path_bytes(&rel);
        restore::restore_deleted(&history, &rel, &fallback_name)
            .map(|path| path.to_string_lossy().to_string())
            .map_err(|e| e.to_string())
    })
    .await?
}

/// Materialize a deleted file's last real version as a read-only cache copy.
#[tauri::command]
async fn open_deleted(app: tauri::AppHandle, path: String, name: String) -> Result<String, String> {
    blocking(move || {
        let (history, rel) = history_and_rel(&app, &path)?;
        let rel = versions::path_bytes(&rel);
        let base = opened_versions_dir(&app)?;
        restore::open_deleted(&history, &rel, &name, &base)
            .map(|path| path.to_string_lossy().to_string())
            .map_err(|e| e.to_string())
    })
    .await?
}

#[tauri::command]
async fn history_info(
    app: tauri::AppHandle,
    root: String,
) -> Result<versions::HistoryInfo, String> {
    blocking(move || {
        app.state::<VersionsState>()
            .info_under(Path::new(&root))
            .map_err(|e| e.to_string())
    })
    .await?
}

#[tauri::command]
async fn clear_history(app: tauri::AppHandle, root: String) -> Result<(), String> {
    blocking(move || {
        app.state::<VersionsState>()
            .clear(Path::new(&root), trash_history_dir)
    })
    .await?
}

#[tauri::command]
async fn stop_keeping_versions(app: tauri::AppHandle, root: String) -> Result<(), String> {
    blocking(move || {
        app.state::<VersionsState>()
            .stop_keeping(Path::new(&root), trash_history_dir)
    })
    .await?
}

/// Clear History and Stop Keeping Versions move `.versions/` to the Trash (D7).
fn trash_history_dir(dir: &Path) -> Result<(), String> {
    trash::delete(dir).map_err(|e| format!("Could not move the version history to the Trash: {e}"))
}

#[tauri::command]
async fn empty_trash(app: tauri::AppHandle) -> Result<(), String> {
    blocking(move || {
        let home = home_dir(&app)?;
        trash_ops::empty(&trash_ops::home_trash(&home))
    })
    .await?
}

fn thumbnail_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_cache_dir()
        .ok()
        .map(|d| d.join("thumbnails"))
}

/// Where opened versions are materialized as read-only copies.
fn opened_versions_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_cache_dir()
        .map(|dir| dir.join("opened-versions"))
        .map_err(|e| e.to_string())
}

/// PNG bytes of a thumbnail for `path`, at most `size` pixels square; empty
/// when the file has no thumbnail. Raw bytes avoid JSON-encoding the image.
#[tauri::command]
async fn thumbnail(app: tauri::AppHandle, path: String, size: u32) -> tauri::ipc::Response {
    let size = size.clamp(32, 1024);
    let png = if cloud::is_cloud(&path) {
        blocking(move || {
            let state = app.state::<CloudState>();
            cloud::service::thumbnail(&state, &path, size)
        })
        .await
        .unwrap_or_default()
    } else {
        blocking(move || {
            thumbnail_dir(&app)
                .and_then(|dir| {
                    thumbnails::thumbnail(Path::new(&path), size, &dir)
                        .ok()
                        .flatten()
                })
                .and_then(|p| std::fs::read(p).ok())
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    };
    tauri::ipc::Response::new(png)
}

/// Mounted volumes for the sidebar: `/Volumes` on macOS, the udisks2 mount
/// folders and `/mnt` on Linux.
#[tauri::command]
async fn volumes() -> Result<Vec<platform::Volume>, String> {
    blocking(move || platform::volumes(&platform::volume_roots())).await
}

#[derive(serde::Serialize)]
struct DriveConnectStart {
    flow_id: String,
    auth_url: String,
}

#[tauri::command]
fn drive_accounts(
    state: tauri::State<CloudState>,
) -> Result<Vec<cloud::accounts::DriveAccount>, String> {
    Ok(lock_state(&state.accounts)?.items().to_vec())
}

#[tauri::command]
fn drive_disconnect(state: tauri::State<CloudState>, account: String) -> Result<(), String> {
    if let Ok(Some(token)) = state.token(&account) {
        cloud::oauth::revoke(token.refresh_token().unwrap_or(&token.access_token));
    }
    state.delete_token(&account)?;
    lock_state(&state.accounts)?.remove(&account)?;
    Ok(())
}

#[tauri::command]
fn drive_connect_start(state: tauri::State<CloudState>) -> Result<DriveConnectStart, String> {
    let flow = cloud::oauth::start(&[cloud::oauth::SCOPE_READONLY])?;
    let start = DriveConnectStart {
        flow_id: flow.id.clone(),
        auth_url: flow.auth_url.clone(),
    };
    let mut flows = lock_state(&state.flows)?;
    // A frontend that vanished mid-dialog must not leave listeners behind.
    flows.retain(|_, flow| {
        flow.created.elapsed() < std::time::Duration::from_secs(300) && !flow.is_cancelled()
    });
    flows.insert(flow.id.clone(), std::sync::Arc::new(flow));
    Ok(start)
}

#[tauri::command]
async fn drive_connect_finish(
    app: tauri::AppHandle,
    flow_id: String,
) -> Result<cloud::accounts::DriveAccount, String> {
    blocking(move || {
        let state = app.state::<CloudState>();
        let flow = lock_state(&state.flows)?
            .get(&flow_id)
            .cloned()
            .ok_or_else(|| "This sign-in attempt is no longer active.".to_string())?;
        flow.claim()?;

        let result = (|| -> Result<cloud::accounts::DriveAccount, String> {
            let token = cloud::oauth::finish(
                &flow,
                &cloud::oauth::GoogleExchange::new(),
                cloud::oauth::CALLBACK_TIMEOUT,
            )?;
            if token.refresh_token.is_none() {
                return Err("Google did not return a refresh token. Sign in again.".into());
            }
            let about = cloud::service::about(&state, &token.access_token)?;
            let scopes = if token.scopes.is_empty() {
                vec![cloud::oauth::SCOPE_READONLY.to_string()]
            } else {
                token.scopes.clone()
            };
            let account = cloud::accounts::DriveAccount::new(
                cloud::accounts::new_account_id(),
                about.email,
                about.name,
                about.root_id,
                scopes,
                recent::now(),
            );
            state.save_token(&account.id, &token)?;
            if let Err(error) = lock_state(&state.accounts)?.upsert(account.clone()) {
                let _ = state.delete_token(&account.id);
                return Err(error);
            }
            Ok(account)
        })();

        let _ = lock_state(&state.flows).map(|mut flows| flows.remove(&flow_id));
        result
    })
    .await?
}

#[tauri::command]
fn drive_connect_cancel(state: tauri::State<CloudState>, flow_id: String) -> Result<(), String> {
    if let Some(flow) = lock_state(&state.flows)?.remove(&flow_id) {
        flow.cancel();
    }
    Ok(())
}

#[tauri::command]
async fn drive_open(
    app: tauri::AppHandle,
    path: String,
) -> Result<cloud::service::OpenTarget, String> {
    blocking(move || {
        let state = app.state::<CloudState>();
        let cache = app
            .path()
            .app_cache_dir()
            .map_err(|error| error.to_string())?;
        cloud::service::open(&state, &path, &cache)
    })
    .await?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    for (key, value) in platform::webview_env_to_set(|k| std::env::var_os(k).is_some()) {
        // SAFETY: still single-threaded; Tauri and the webview start below.
        unsafe { std::env::set_var(key, value) };
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| PathBuf::from("."));
            app.manage(VersionsState::new(config_dir.join("versioned.json")));
            app.manage(CloudState::new(&config_dir));
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                handle.state::<VersionsState>().startup();
            });
            // Drop thumbnails nobody has needed for two months.
            if let Some(dir) = thumbnail_dir(app.handle()) {
                std::thread::spawn(move || {
                    thumbnails::prune(&dir, std::time::Duration::from_secs(60 * 86_400))
                });
            }
            // Drop read-only copies opened from Versions… after a week.
            if let Ok(dir) = opened_versions_dir(app.handle()) {
                std::thread::spawn(move || {
                    restore::prune_opened(&dir, versions::now_seconds(), 7 * 86_400);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_dir,
            item_counts,
            make_dir,
            rename_entry,
            fs_ops::path_exists,
            path_info,
            file_properties,
            folder_size,
            new_folder_walk,
            cancel_folder_size,
            copy_entry,
            move_entry,
            trash_entry,
            trash_info,
            restore_entry,
            delete_permanently,
            empty_trash,
            trash_dir,
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
            volumes,
            drive_accounts,
            drive_disconnect,
            drive_connect_start,
            drive_connect_finish,
            drive_connect_cancel,
            drive_open,
            make_versioned_dir,
            versioned_root,
            list_versions,
            version_count,
            version_diff,
            restore_version_copy,
            open_version,
            replace_with_version,
            deleted_files,
            restore_deleted,
            open_deleted,
            history_info,
            clear_history,
            stop_keeping_versions,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

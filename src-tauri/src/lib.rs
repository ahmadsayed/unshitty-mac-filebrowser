mod archive;
mod cloud;
mod config_store;
mod downloads;
mod file_icons;
mod fs_ops;
mod platform;
mod progress;
mod properties;
mod recent;
mod sftp;
mod share;
mod thumbnails;
mod trash_ops;
mod updates;
mod versions;

use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use progress::ProgressUpdate;
use tauri::ipc::Channel;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

use cloud::CloudState;
use versions::history::History;
use versions::state::{Transfer, VersionsState};
use versions::{VersionDiff, VersionInfo, restore};

fn config_file(app: &tauri::AppHandle, name: &str) -> Result<PathBuf, String> {
    let dir = match share::identity::config_dir_override() {
        Some(dir) => dir,
        None => app.path().app_config_dir().map_err(|e| e.to_string())?,
    };
    Ok(dir.join(name))
}

fn lock_state<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, String> {
    mutex
        .lock()
        .map_err(|_| "Google Drive state is unavailable.".to_string())
}

fn read_only_one(path: &str) -> Option<&'static str> {
    if sftp::is_sftp(path) {
        return Some("SSH servers are read only for now.");
    }
    if share::location::is_share(path) {
        return Some("Shared folders are read only.");
    }
    None
}

fn read_only_write(from: &str, to: &str) -> Option<&'static str> {
    read_only_one(from).or_else(|| read_only_one(to))
}

fn share_versions_hint(app: &tauri::AppHandle) -> share::host::VersionsHint {
    let app = app.clone();
    Box::new(move |path, size, mtime| {
        app.state::<VersionsState>()
            .content_hint(path, size, mtime)
    })
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

// ---------------------------------------------------------------------------
// Operation progress (spec 2026-10-25-operation-progress, D3)
// ---------------------------------------------------------------------------

/// How often a running operation reports its progress.
const PROGRESS_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

/// One command's part of operation `op`: its progress, registered so
/// `cancel_operation` reaches it, and a thread streaming it to the frontend.
struct Tracking {
    op: String,
    progress: progress::Progress,
    stop: std::sync::mpsc::Sender<()>,
    ticker: std::thread::JoinHandle<()>,
}

impl Tracking {
    fn start(app: &tauri::AppHandle, op: &str, channel: Channel<ProgressUpdate>) -> Self {
        let progress = app.state::<progress::Registry>().register(op);
        progress.set_phase(progress::Phase::Working);
        let (stop, stopped) = std::sync::mpsc::channel();
        let ticker = {
            let progress = progress.clone();
            std::thread::spawn(move || {
                let send = |update| {
                    let _ = channel.send(update);
                };
                progress::ticker(&progress, send, &stopped, PROGRESS_EVERY);
            })
        };
        Tracking {
            op: op.to_string(),
            progress,
            stop,
            ticker,
        }
    }

    /// Stop the ticker (it sends a last update with `done`) and forget `op`.
    fn finish(self, app: &tauri::AppHandle) {
        drop(self.stop);
        let _ = self.ticker.join();
        app.state::<progress::Registry>().remove(&self.op);
    }
}

/// Cancel operation `op`; its current command ends with "Cancelled.".
#[tauri::command]
fn cancel_operation(registry: tauri::State<progress::Registry>, op: String) {
    registry.cancel(&op);
}

/// The size and item count of `paths` (local and Drive), for a progress total.
#[tauri::command]
async fn measure(app: tauri::AppHandle, paths: Vec<String>) -> Result<fs_ops::Measure, String> {
    blocking(move || {
        let (drive, local): (Vec<String>, Vec<String>) =
            paths.into_iter().partition(|path| cloud::is_cloud(path));
        let local: Vec<PathBuf> = local.into_iter().map(PathBuf::from).collect();
        let mut total = fs_ops::measure(&local);
        if !drive.is_empty() {
            let remote = cloud::transfer::measure(&app.state::<CloudState>(), &drive)?;
            total.bytes += remote.bytes;
            total.items += remote.items;
        }
        Ok(total)
    })
    .await?
}

#[tauri::command]
async fn list_dir(
    app: tauri::AppHandle,
    path: String,
    show_hidden: bool,
) -> Result<Vec<fs_ops::Entry>, String> {
    if share::location::is_share(&path) {
        let state = app.state::<share::state::ShareState>();
        let hint = share_versions_hint(&app);
        return state.list_dir(&path, show_hidden, hint).await;
    }
    if sftp::is_sftp(&path) {
        let app = app.clone();
        return blocking(move || app.state::<sftp::SftpState>().list(&path, show_hidden)).await?;
    }
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
async fn sftp_connect(
    app: tauri::AppHandle,
    url: String,
    password: Option<String>,
    trust_host: bool,
) -> Result<sftp::ConnectOutcome, String> {
    let app = app.clone();
    blocking(move || {
        app.state::<sftp::SftpState>()
            .connect(&url, password, trust_host)
    })
    .await?
}

#[tauri::command]
fn sftp_servers(app: tauri::AppHandle) -> Vec<sftp::ServerInfo> {
    app.state::<sftp::SftpState>().servers()
}

#[tauri::command]
fn sftp_disconnect(app: tauri::AppHandle, id: String) {
    app.state::<sftp::SftpState>().disconnect(&id);
}

#[tauri::command]
async fn sftp_open(
    app: tauri::AppHandle,
    path: String,
    op: String,
    on_progress: Channel<ProgressUpdate>,
) -> Result<String, String> {
    let tracking = Tracking::start(&app, &op, on_progress);
    let progress = tracking.progress.clone();
    let state = app.clone();
    let result = blocking(move || state.state::<sftp::SftpState>().open(&path, &progress)).await;
    tracking.finish(&app);
    result?
}

/// The search running in each window: starting another, or `cancel_search`,
/// stops it, so typing doesn't leave old searches walking the disk.
#[derive(Default)]
struct Searches(Mutex<HashMap<String, Arc<AtomicBool>>>);

impl Searches {
    /// Stop `window`'s running search and return the flag for its next one.
    fn restart(&self, window: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        let mut running = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = running.insert(window.to_string(), flag.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        flag
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn search_dir(
    app: tauri::AppHandle,
    window: tauri::Window,
    path: String,
    query: String,
    show_hidden: bool,
    limit: usize,
    on_results: Channel<Vec<fs_ops::Entry>>,
) -> Result<fs_ops::SearchSummary, String> {
    let stop = app.state::<Searches>().restart(window.label());
    blocking(move || {
        fs_ops::search_dir(
            &path,
            &query,
            show_hidden,
            limit,
            &mut |batch| {
                let _ = on_results.send(batch);
            },
            &|| stop.load(Ordering::Relaxed),
        )
    })
    .await?
}

#[tauri::command]
fn cancel_search(app: tauri::AppHandle, window: tauri::Window) {
    app.state::<Searches>().restart(window.label());
}

#[tauri::command]
async fn extract_archive(
    path: String,
    dest: String,
    create: bool,
) -> Result<archive::ExtractResult, String> {
    blocking(move || archive::extract_archive(path, dest, create)).await?
}

#[tauri::command]
async fn compress_archive(paths: Vec<String>, dest: String, format: String) -> Result<(), String> {
    blocking(move || archive::compress_archive(paths, dest, format)).await?
}

static WINDOW_SEQ: AtomicU64 = AtomicU64::new(1);

#[tauri::command]
async fn window_buttons() -> Result<platform::WindowButtons, String> {
    blocking(platform::window_buttons).await
}

/// The header bar's window buttons (see `platform::window_control`).
#[tauri::command]
fn window_control(window: tauri::WebviewWindow, action: String) -> Result<bool, String> {
    platform::window_control(&window, &action)
}

impl platform::WindowOps for tauri::WebviewWindow {
    fn minimize(&self) -> Result<(), String> {
        tauri::WebviewWindow::minimize(self).map_err(|e| e.to_string())
    }
    fn maximize(&self) -> Result<(), String> {
        tauri::WebviewWindow::maximize(self).map_err(|e| e.to_string())
    }
    fn unmaximize(&self) -> Result<(), String> {
        tauri::WebviewWindow::unmaximize(self).map_err(|e| e.to_string())
    }
    fn close(&self) -> Result<(), String> {
        tauri::WebviewWindow::close(self).map_err(|e| e.to_string())
    }
    fn is_maximized(&self) -> Result<bool, String> {
        tauri::WebviewWindow::is_maximized(self).map_err(|e| e.to_string())
    }
}

#[tauri::command]
async fn new_window(app: tauri::AppHandle, path: Option<String>) -> Result<(), String> {
    let label = format!("files-{}", WINDOW_SEQ.fetch_add(1, Ordering::Relaxed));
    let url = match path.filter(|path| !path.is_empty()) {
        Some(path) => {
            let encoded = percent_encoding::utf8_percent_encode(
                &path,
                percent_encoding::NON_ALPHANUMERIC,
            );
            WebviewUrl::App(format!("index.html?path={encoded}").into())
        }
        None => WebviewUrl::App("index.html".into()),
    };
    WebviewWindowBuilder::new(&app, label, url)
        .title("Files")
        .inner_size(1100.0, 720.0)
        .min_inner_size(640.0, 420.0)
        // On Linux the header bars are the title bar (see tauri.linux.conf.json).
        .decorations(!cfg!(target_os = "linux"))
        .focused(true)
        .build()
        .map(|_| ())
        .map_err(|e| format!("Could not open a new window: {e}"))
}

#[tauri::command]
async fn list_apps() -> Result<Vec<platform::AppChoice>, String> {
    blocking(platform::applications).await
}

#[tauri::command]
async fn open_file(path: String) -> Result<(), String> {
    blocking(move || platform::open_default(&path)).await?
}

/// Open a web link (or, on a Mac, the Full Disk Access settings) through the
/// same launcher as files, so it gets the host's environment.
#[tauri::command]
async fn open_url(url: String) -> Result<(), String> {
    blocking(move || platform::open_url(&url)).await?
}

#[tauri::command]
async fn open_with(path: String, app: String) -> Result<(), String> {
    blocking(move || platform::open_with(&path, &app)).await?
}

#[tauri::command]
async fn preview_text(path: String, limit: usize) -> Result<fs_ops::TextPreview, String> {
    blocking(move || fs_ops::preview_text(path, limit)).await?
}

#[tauri::command]
async fn rename_many(items: Vec<fs_ops::RenameItem>) -> Result<(), String> {
    blocking(move || fs_ops::rename_many(items)).await?
}

#[tauri::command]
async fn set_permissions(path: String, mode: u32) -> Result<(), String> {
    blocking(move || fs_ops::set_permissions(path, mode)).await?
}

#[tauri::command]
async fn create_symlink(path: String) -> Result<String, String> {
    blocking(move || fs_ops::create_link(path)).await?
}

#[tauri::command]
async fn create_file(path: String) -> Result<(), String> {
    blocking(move || fs_ops::create_file(path)).await?
}

#[tauri::command]
async fn list_templates(app: tauri::AppHandle) -> Result<Vec<fs_ops::Entry>, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let dir = home.join("Templates");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let path = dir.to_string_lossy().to_string();
    blocking(move || fs_ops::list_dir(path, false)).await?
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
            if !cloud::is_cloud(path) && !share::location::is_share(path) && !sftp::is_sftp(path) {
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
            if share::location::is_share(path) || sftp::is_sftp(path) {
                out.push(None);
            } else if cloud::is_cloud(path) {
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
    if sftp::is_sftp(&path) {
        return Err("Properties aren't available for SSH servers yet.".into());
    }
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
    if sftp::is_sftp(&path) {
        return Err("Folder sizes aren't available for SSH servers yet.".into());
    }
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

/// The write commands return the item's final path: for Drive it is only
/// known once Drive has given the new item an id.
#[tauri::command]
async fn make_dir(app: tauri::AppHandle, path: String) -> Result<String, String> {
    if let Some(message) = read_only_one(&path) {
        return Err(message.into());
    }
    if cloud::is_cloud(&path) {
        return blocking(move || cloud::write::make_dir(&app.state::<CloudState>(), &path)).await?;
    }
    fs_ops::make_dir(path.clone())?;
    Ok(path)
}

/// Copy or move when either side is in Drive. A local source of a move goes
/// to the Trash the way Move to Trash sends it, after the upload succeeded.
async fn drive_transfer(
    app: tauri::AppHandle,
    from: String,
    to: String,
    overwrite: bool,
    moving: bool,
    progress: progress::Progress,
) -> Result<String, String> {
    blocking(move || {
        let home = home_dir(&app)?;
        let versions = app.state::<VersionsState>();
        let trash_local = |source: &Path| {
            versions
                .trash(source, || trash_ops::trash_and_locate(source, &home))
                .map(|_| ())
        };
        let state = app.state::<CloudState>();
        cloud::transfer::dispatch(
            &state,
            &from,
            &to,
            overwrite,
            moving,
            &progress,
            trash_local,
        )
    })
    .await?
}

#[tauri::command]
async fn path_exists(app: tauri::AppHandle, path: String) -> bool {
    blocking(move || {
        if cloud::is_cloud(&path) {
            cloud::service::path_exists(&app.state::<CloudState>(), &path)
        } else {
            fs_ops::path_exists(path)
        }
    })
    .await
    .unwrap_or(false)
}

#[tauri::command]
async fn copy_entry(
    app: tauri::AppHandle,
    from: String,
    to: String,
    overwrite: bool,
    op: String,
    on_progress: Channel<ProgressUpdate>,
) -> Result<String, String> {
    if let Some(message) = read_only_write(&from, &to) {
        return Err(message.into());
    }
    let tracking = Tracking::start(&app, &op, on_progress);
    let progress = tracking.progress.clone();
    let result = if cloud::is_cloud(&from) || cloud::is_cloud(&to) {
        drive_transfer(app.clone(), from, to, overwrite, false, progress).await
    } else {
        let app = app.clone();
        blocking(move || {
            let state = app.state::<VersionsState>();
            let (from_path, to_path) = (PathBuf::from(&from), PathBuf::from(&to));
            state.transfer(&from_path, &to_path, Transfer::Copy, || {
                fs_ops::copy_entry(from, to.clone(), overwrite, &progress)
            })?;
            Ok(to_path.to_string_lossy().to_string())
        })
        .await
        .and_then(|result| result)
    };
    tracking.finish(&app);
    result
}

#[tauri::command]
async fn move_entry(
    app: tauri::AppHandle,
    from: String,
    to: String,
    overwrite: bool,
    op: String,
    on_progress: Channel<ProgressUpdate>,
) -> Result<String, String> {
    if let Some(message) = read_only_write(&from, &to) {
        return Err(message.into());
    }
    let tracking = Tracking::start(&app, &op, on_progress);
    let progress = tracking.progress.clone();
    let result = if cloud::is_cloud(&from) || cloud::is_cloud(&to) {
        drive_transfer(app.clone(), from, to, overwrite, true, progress).await
    } else {
        let app = app.clone();
        blocking(move || {
            let state = app.state::<VersionsState>();
            let (from_path, to_path) = (PathBuf::from(&from), PathBuf::from(&to));
            state.transfer(&from_path, &to_path, Transfer::Move, || {
                fs_ops::move_entry(from, to.clone(), overwrite, &progress)
            })?;
            Ok(to_path.to_string_lossy().to_string())
        })
        .await
        .and_then(|result| result)
    };
    tracking.finish(&app);
    result
}

#[tauri::command]
async fn rename_entry(app: tauri::AppHandle, from: String, to: String) -> Result<String, String> {
    if let Some(message) = read_only_write(&from, &to) {
        return Err(message.into());
    }
    // write::rename refuses a local path on either side.
    if cloud::is_cloud(&from) || cloud::is_cloud(&to) {
        return blocking(move || cloud::write::rename(&app.state::<CloudState>(), &from, &to))
            .await?;
    }
    blocking(move || {
        let state = app.state::<VersionsState>();
        let (from_path, to_path) = (PathBuf::from(&from), PathBuf::from(&to));
        state.transfer(&from_path, &to_path, Transfer::Move, || {
            fs_ops::rename_entry(from, to.clone())
        })?;
        Ok(to_path.to_string_lossy().to_string())
    })
    .await?
}

/// Serializes read-modify-write of `trash_log.json`.
static TRASH_LOG: Mutex<()> = Mutex::new(());

fn home_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path().home_dir().map_err(|e| e.to_string())
}

/// One path of a Move to Trash: where it went, when that's known (so the
/// move can be undone), or why it is still there.
#[derive(serde::Serialize)]
struct TrashOutcome {
    trashed: Option<String>,
    error: Option<String>,
}

/// Move `paths` to the Trash as one batch: one Finder request (and one
/// sound) on macOS rather than one per item, counted in items for the
/// operation's progress. Returns one outcome per path, in order.
#[tauri::command]
async fn trash_entries(
    app: tauri::AppHandle,
    paths: Vec<String>,
    op: String,
    on_progress: Channel<ProgressUpdate>,
) -> Result<Vec<TrashOutcome>, String> {
    let tracking = Tracking::start(&app, &op, on_progress);
    let progress = tracking.progress.clone();
    let worker = app.clone();
    let outcomes = blocking(move || trash_batch(&worker, &paths, &progress)).await;
    tracking.finish(&app);
    outcomes
}

fn trash_batch(
    app: &tauri::AppHandle,
    paths: &[String],
    progress: &progress::Progress,
) -> Vec<TrashOutcome> {
    let home = home_dir(app);
    let versions = app.state::<VersionsState>();
    let local = |origins: &[PathBuf]| match &home {
        Ok(home) => versions.trash_many(origins, |ready| trash_ops::trash_many(ready, home, progress)),
        Err(error) => origins.iter().map(|_| Err(error.clone())).collect(),
    };
    let (results, moves) =
        trash_ops::trash_routed(paths, |path| trash_elsewhere(app, path, progress), local);
    log_trashed(app, &moves);
    results.into_iter().map(TrashOutcome::from).collect()
}

/// A path of a Move to Trash that doesn't go to a local Trash: refused for
/// read-only locations, or sent to Drive's own Trash (D10; never a
/// permanent delete). `None` for local paths.
fn trash_elsewhere(
    app: &tauri::AppHandle,
    path: &str,
    progress: &progress::Progress,
) -> Option<Result<Option<String>, String>> {
    if let Some(message) = read_only_one(path) {
        return Some(Err(message.into()));
    }
    if !cloud::is_cloud(path) {
        return None;
    }
    let result = progress
        .check()
        .and_then(|()| cloud::write::trash(&app.state::<CloudState>(), path));
    if result.is_ok() {
        progress.add_item();
    }
    Some(result)
}

/// Remember where trashed items went, for Undo and Restore.
fn log_trashed(app: &tauri::AppHandle, moves: &[(PathBuf, PathBuf)]) {
    let Ok(log_path) = config_file(app, "trash_log.json") else {
        return;
    };
    if moves.is_empty() {
        return;
    }
    let _guard = TRASH_LOG.lock().unwrap_or_else(|e| e.into_inner());
    let mut log = trash_ops::read_log(&log_path);
    trash_ops::record(&mut log, moves, recent::now());
    let _ = config_store::write_json(&log_path, &log);
}

impl From<Result<Option<String>, String>> for TrashOutcome {
    fn from(result: Result<Option<String>, String>) -> Self {
        match result {
            Ok(trashed) => TrashOutcome { trashed, error: None },
            Err(error) => TrashOutcome { trashed: None, error: Some(error) },
        }
    }
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
        return blocking(move || cloud::write::restore(&app.state::<CloudState>(), &path)).await?;
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

fn mac_dmg_path(app: &tauri::AppHandle, path: &str) -> Result<PathBuf, String> {
    if !platform::IS_MACOS { return Err("DMG installation is available on macOS only.".into()); }
    let original = Path::new(path);
    if !original.is_absolute() { return Err("Choose a disk image on this computer.".into()); }
    let canonical = original.canonicalize().map_err(|e| e.to_string())?;
    if [canonical.as_path(), original].iter().any(|p| in_trash(app, p)) {
        return Err("Restore the disk image from the Trash before installing it.".into());
    }
    Ok(original.to_path_buf())
}

fn linux_program_path(app: &tauri::AppHandle, path: &str) -> Result<PathBuf, String> {
    if !cfg!(target_os = "linux") { return Err("These program actions are available on Linux only.".into()); }
    let original = Path::new(path);
    if !original.is_absolute() { return Err("Choose a file on this computer.".into()); }
    let canonical = original.canonicalize().map_err(|e| e.to_string())?;
    let trashed = [canonical.as_path(), original].iter().any(|p| in_trash(app, p));
    if trashed {
        return Err("Restore this file from the Trash before installing or running it.".into());
    }
    // Preserve the original spelling so platform code can refuse a symlink.
    Ok(original.to_path_buf())
}

#[tauri::command]
async fn inspect_dmg(app: tauri::AppHandle, path: String) -> Result<platform::DmgInspection, String> {
    blocking(move || {
        let source = mac_dmg_path(&app, &path)?;
        let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("dmg-installs");
        platform::inspect_dmg(&source, &cache)
    }).await?
}

#[tauri::command]
async fn install_appimage(app: tauri::AppHandle, path: String, name: String) -> Result<platform::InstalledAppImage, String> {
    blocking(move || {
        let source = linux_program_path(&app, &path)?;
        let home = app.path().home_dir().map_err(|e| e.to_string())?;
        let data = platform::data_home(&home);
        let installed = platform::install_appimage_at(&source, &name, &data)?;
        platform::refresh_application_cache(&data);
        Ok(installed)
    }).await?
}

#[tauri::command]
async fn install_dmg(app: tauri::AppHandle, path: String, fingerprint: String, application: String, destination: String, mode: platform::DmgInstallMode) -> Result<platform::DmgInstallResult, String> {
    blocking(move || {
        let image = mac_dmg_path(&app, &path)?;
        let home = app.path().home_dir().map_err(|e| e.to_string())?;
        let directory = platform::dmg_application_directory(&home, &destination)?;
        let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("dmg-installs");
        let request = platform::DmgInstallRequest { image, fingerprint, application, directory, mode };
        let versions = app.state::<VersionsState>();
        platform::install_dmg_application(&request, &cache, &|old| {
            versions.trash(old, || trash::delete(old).map_err(|e| e.to_string()))
        })
    }).await?
}

#[tauri::command]
async fn run_shell_script(app: tauri::AppHandle, path: String) -> Result<(), String> {
    blocking(move || platform::run_shell_script(&linux_program_path(&app, &path)?)).await?
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
    if cloud::is_cloud(&path) || share::location::is_share(&path) {
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
async fn empty_trash(
    app: tauri::AppHandle,
    op: String,
    on_progress: Channel<ProgressUpdate>,
) -> Result<(), String> {
    let tracking = Tracking::start(&app, &op, on_progress);
    let progress = tracking.progress.clone();
    let home = home_dir(&app);
    let result = blocking(move || trash_ops::empty(&trash_ops::home_trash(&home?), &progress)).await;
    tracking.finish(&app);
    result?
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
/// when the file has no thumbnail or the request was cancelled. Raw bytes
/// avoid JSON-encoding the image. `ticket`, when given, names the request
/// for `cancel_thumbnails`.
#[tauri::command]
async fn thumbnail(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    path: String,
    size: u32,
    ticket: Option<u64>,
) -> tauri::ipc::Response {
    let size = size.clamp(32, 1024);
    let png = if cloud::is_cloud(&path) {
        blocking(move || {
            let state = app.state::<CloudState>();
            cloud::service::thumbnail(&state, &path, size)
        })
        .await
        .unwrap_or_default()
    } else if share::location::is_share(&path) || sftp::is_sftp(&path) {
        Vec::new()
    } else {
        // Registered until it finishes, so the window can cancel it.
        let request = ticket.map(|t| app.state::<thumbnails::Requests>().start(window.label(), t));
        let stop = request.as_ref().map(thumbnails::Request::stop_flag);
        blocking(move || {
            let stop = || stop.as_ref().is_some_and(|s| s.load(std::sync::atomic::Ordering::SeqCst));
            thumbnail_dir(&app)
                .and_then(|dir| {
                    thumbnails::thumbnail_unless(Path::new(&path), size, &dir, &stop)
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

/// Start sharing one local file with a QR code; the id names the share for
/// `file_share_status` and `file_share_stop`. See `share::file_share`.
#[tauri::command]
async fn file_share_prepare(
    shares: tauri::State<'_, share::file_share::FileShares>,
    path: String,
) -> Result<String, String> {
    if cloud::is_cloud(&path) || share::location::is_share(&path) || sftp::is_sftp(&path) {
        return Err("Only files on this computer can be shared with a QR code.".into());
    }
    shares.prepare(Path::new(&path))
}

/// Progress, link and downloads of a QR share; None once it was stopped.
#[tauri::command]
fn file_share_status(
    shares: tauri::State<'_, share::file_share::FileShares>,
    id: String,
) -> Option<share::file_share::View> {
    shares.status(&id)
}

/// Stop a QR share: phones that ask afterwards are told it ended.
#[tauri::command]
fn file_share_stop(shares: tauri::State<'_, share::file_share::FileShares>, id: String) {
    shares.stop(&id);
}

/// The icon id the OS has for each path (None: the app draws its own icon);
/// see `file_icons`. Remote locations have none.
#[tauri::command]
async fn icon_ids(app: tauri::AppHandle, paths: Vec<String>) -> Vec<Option<String>> {
    blocking(move || {
        let icons = app.state::<file_icons::Icons>();
        paths
            .iter()
            .map(|path| {
                let remote = cloud::is_cloud(path) || share::location::is_share(path) || sftp::is_sftp(path);
                if remote { None } else { icons.id(Path::new(path)) }
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// The icon id for each app Open With offers (`list_apps` ids), or None.
#[tauri::command]
async fn app_icon_ids(app: tauri::AppHandle, apps: Vec<String>) -> Vec<Option<String>> {
    blocking(move || {
        let icons = app.state::<file_icons::Icons>();
        apps.iter().map(|id| icons.app_id(Path::new(id))).collect()
    })
    .await
    .unwrap_or_default()
}

/// The image for an id from `icon_ids`, about `size` pixels square: PNG or
/// SVG bytes, empty when there is none.
#[tauri::command]
async fn icon_image(app: tauri::AppHandle, id: String, size: u32) -> tauri::ipc::Response {
    let size = size.clamp(16, 1024);
    let bytes = blocking(move || app.state::<file_icons::Icons>().image(&id, size))
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    tauri::ipc::Response::new(bytes)
}

/// The icon theme the desktop uses, read from GTK on the main thread at
/// start-up (GTK objects can't be used from the command threads).
#[cfg(target_os = "linux")]
fn desktop_icon_theme() -> Option<String> {
    use gtk::prelude::*;
    gtk::Settings::default()?.property::<Option<String>>("gtk-icon-theme-name")
}

#[cfg(not(target_os = "linux"))]
fn desktop_icon_theme() -> Option<String> {
    None
}

/// Cancel this window's thumbnail requests with these tickets: the items
/// scrolled away or the folder was left.
#[tauri::command]
fn cancel_thumbnails(
    window: tauri::WebviewWindow,
    requests: tauri::State<thumbnails::Requests>,
    tickets: Vec<u64>,
) {
    requests.cancel(window.label(), &tickets);
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

#[derive(serde::Serialize)]
struct DriveClientStatus {
    configured: bool,
    client_id: Option<String>,
}

/// Whether the user has added their OAuth client. The secret never leaves the
/// backend.
#[tauri::command]
fn drive_client_status(state: tauri::State<CloudState>) -> DriveClientStatus {
    let client = state.oauth_client();
    DriveClientStatus {
        configured: client.is_some(),
        client_id: client.map(|client| client.client_id),
    }
}

/// Store the user's OAuth client from the JSON file Google lets them download.
#[tauri::command]
fn drive_client_set(
    state: tauri::State<CloudState>,
    json: String,
) -> Result<DriveClientStatus, String> {
    let client = state.set_oauth_client(&json)?;
    Ok(DriveClientStatus {
        configured: true,
        client_id: Some(client.client_id),
    })
}

#[tauri::command]
fn drive_client_clear(state: tauri::State<CloudState>) -> Result<(), String> {
    state.clear_oauth_client()
}

#[tauri::command]
fn drive_connect_start(state: tauri::State<CloudState>) -> Result<DriveConnectStart, String> {
    let flow = cloud::oauth::start(&state.require_oauth_client()?, &[cloud::oauth::SCOPE_FULL])?;
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
                &cloud::oauth::GoogleExchange::new(&state.require_oauth_client()?),
                cloud::oauth::CALLBACK_TIMEOUT,
            )?;
            if token.refresh_token.is_none() {
                return Err("Google did not return a refresh token. Sign in again.".into());
            }
            let about = cloud::service::about(&state, &token.access_token)?;
            let scopes = if token.scopes.is_empty() {
                vec![cloud::oauth::SCOPE_FULL.to_string()]
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
    op: String,
    on_progress: Channel<ProgressUpdate>,
) -> Result<cloud::service::OpenTarget, String> {
    let tracking = Tracking::start(&app, &op, on_progress);
    let progress = tracking.progress.clone();
    let handle = app.clone();
    let result = blocking(move || {
        let state = handle.state::<CloudState>();
        let cache = handle
            .path()
            .app_cache_dir()
            .map_err(|error| error.to_string())?;
        cloud::service::open(&state, &path, &cache, &progress)
    })
    .await;
    tracking.finish(&app);
    result?
}

#[tauri::command]
fn share_identity(
    state: tauri::State<share::state::ShareState>,
) -> Result<share::state::ShareIdentity, String> {
    state.identity()
}

#[tauri::command]
fn set_share_name(
    state: tauri::State<share::state::ShareState>,
    name: String,
) -> Result<(), String> {
    state.set_name((!name.trim().is_empty()).then_some(name.trim().to_string()))
}

#[tauri::command]
async fn share_folder(
    app: tauri::AppHandle,
    state: tauri::State<'_, share::state::ShareState>,
    path: String,
) -> Result<share::state::ShareInfo, String> {
    let hint = share_versions_hint(&app);
    let trash = app.clone();
    state
        .share_folder(Path::new(&path), move |p| in_trash(&trash, p), hint)
        .await
}

#[tauri::command]
async fn share_info(
    app: tauri::AppHandle,
    state: tauri::State<'_, share::state::ShareState>,
    path: String,
) -> Result<Option<share::state::ShareInfo>, String> {
    let hint = share_versions_hint(&app);
    state.info(Path::new(&path), hint).await
}

#[tauri::command]
async fn list_shares(
    app: tauri::AppHandle,
    state: tauri::State<'_, share::state::ShareState>,
) -> Result<Vec<share::state::ShareSummary>, String> {
    state.shares(share_versions_hint(&app)).await
}

#[tauri::command]
async fn create_link(
    app: tauri::AppHandle,
    state: tauri::State<'_, share::state::ShareState>,
    share: String,
    kind: String,
    person: Option<String>,
    passphrase: Option<String>,
) -> Result<share::state::LinkResult, String> {
    state
        .create_link(&share, &kind, person, passphrase, share_versions_hint(&app))
        .await
}

#[tauri::command]
fn share_link_requires_passphrase(link: String) -> Result<bool, String> {
    share::link::ShareLink::requires_passphrase(&link)
}

#[tauri::command]
async fn share_link(
    app: tauri::AppHandle,
    state: tauri::State<'_, share::state::ShareState>,
    share: String,
    entry: String,
) -> Result<String, String> {
    state
        .link(&share, &entry, share_versions_hint(&app))
        .await
}

#[tauri::command]
async fn remove_access(
    app: tauri::AppHandle,
    state: tauri::State<'_, share::state::ShareState>,
    share: String,
    entry: String,
) -> Result<(), String> {
    state
        .remove_access(&share, &entry, share_versions_hint(&app))
        .await
}

#[tauri::command]
async fn stop_sharing(
    app: tauri::AppHandle,
    state: tauri::State<'_, share::state::ShareState>,
    share: String,
) -> Result<(), String> {
    state
        .stop_sharing(&share, share_versions_hint(&app))
        .await
}

#[tauri::command]
async fn add_received(
    app: tauri::AppHandle,
    state: tauri::State<'_, share::state::ShareState>,
    link: String,
    passphrase: Option<String>,
) -> Result<share::state::ReceivedShare, String> {
    state
        .add_received(&link, passphrase, share_versions_hint(&app))
        .await
}

#[tauri::command]
async fn list_received(
    app: tauri::AppHandle,
    state: tauri::State<'_, share::state::ShareState>,
) -> Result<Vec<share::state::ReceivedShare>, String> {
    state.received(share_versions_hint(&app)).await
}

#[tauri::command]
fn remove_received(
    state: tauri::State<share::state::ShareState>,
    id: String,
) -> Result<(), String> {
    state.remove_received(&id)
}

#[tauri::command]
async fn share_open(
    app: tauri::AppHandle,
    state: tauri::State<'_, share::state::ShareState>,
    path: String,
    op: String,
    on_progress: Channel<ProgressUpdate>,
) -> Result<String, String> {
    let tracking = Tracking::start(&app, &op, on_progress);
    let result = state
        .open(&path, share_versions_hint(&app), &tracking.progress)
        .await;
    tracking.finish(&app);
    result
}

// ---------------------------------------------------------------------------
// Check for Updates: updates.rs decides, tauri-plugin-updater downloads and
// verifies against the public key in tauri.conf.json
// ---------------------------------------------------------------------------

const RELEASES_URL: &str = "https://github.com/ahmadsayed/unshitty-mac-filebrowser/releases";

/// The update found by the last check, and the download that may be running.
#[derive(Default)]
struct UpdateState {
    guard: updates::InstallGuard,
    pending: Mutex<Option<tauri_plugin_updater::Update>>,
    /// Whether Cancel may still stop the install (see `DownloadControl`).
    download: Mutex<updates::DownloadControl<DownloadStop>>,
    /// The window whose Download and Install is running: only its Cancel
    /// stops it.
    owner: Mutex<Option<String>>,
    /// The version installed in this run, waiting for a restart.
    installed: Mutex<Option<String>>,
}

/// Stops a download running on a blocking thread (`cancel_update`). A stop
/// that comes before the download waits for it still counts.
#[derive(Clone, Default)]
struct DownloadStop(std::sync::Arc<tokio::sync::Notify>);

impl DownloadStop {
    fn abort(&self) {
        self.0.notify_one();
    }
}

#[derive(serde::Serialize)]
struct UpdateInfo {
    current: String,
    latest: Option<String>,
    kind: updates::InstallKind,
    release_url: String,
    /// `latest` is already installed and waits for a restart.
    restart_pending: bool,
}

#[derive(serde::Serialize, Clone)]
struct UpdateProgress {
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
}

fn current_install_kind() -> updates::InstallKind {
    let appimage = std::env::var("APPIMAGE").ok();
    updates::install_kind(cfg!(debug_assertions), std::env::consts::OS, appimage.as_deref())
}

fn update_failed(detail: &str) -> updates::Failure {
    updates::classify(updates::FailureSource::Other, detail.to_string())
}

/// Sort an updater error into what the dialogs tell apart.
fn failure_from(e: tauri_plugin_updater::Error) -> updates::Failure {
    use tauri_plugin_updater::Error as E;
    use updates::FailureSource;
    let source = match &e {
        E::Reqwest(r) => FailureSource::Network { connect: r.is_connect() || r.is_timeout() },
        E::Network(_) => FailureSource::Network { connect: false },
        E::Minisign(_)
        | E::Base64(_)
        | E::SignatureUtf8(_)
        | E::MissingSignedVersion
        | E::SignedVersionMismatch { .. } => FailureSource::Signature,
        E::Io(io) => FailureSource::Io(io.kind()),
        _ => FailureSource::Other,
    };
    updates::classify(source, e.to_string())
}

/// Ask GitHub for the newest published release. Only called from the
/// Check for Updates dialog: the app never checks on its own.
#[tauri::command]
async fn check_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, UpdateState>,
) -> Result<UpdateInfo, updates::Failure> {
    use tauri_plugin_updater::UpdaterExt;
    let found = app.updater().map_err(failure_from)?.check().await.map_err(failure_from)?;
    let latest = found.as_ref().map(|update| update.version.clone());
    *state.pending.lock().unwrap() = found;
    let restart_pending = updates::restart_pending(state.installed.lock().unwrap().as_deref(), latest.as_deref());
    Ok(UpdateInfo {
        restart_pending,
        current: app.package_info().version.to_string(),
        release_url: match &latest {
            Some(version) => format!("{RELEASES_URL}/tag/v{version}"),
            None => format!("{RELEASES_URL}/latest"),
        },
        latest,
        kind: current_install_kind(),
    })
}

/// Download, verify and install the update the last check found. The
/// download can be cancelled; installing, once started, runs to the end.
#[tauri::command]
async fn install_update(
    window: tauri::Window,
    state: tauri::State<'_, UpdateState>,
    on_progress: Channel<UpdateProgress>,
) -> Result<(), updates::Failure> {
    if !updates::can_self_update(current_install_kind()) {
        return Err(update_failed("This copy can't update itself."));
    }
    let Some(update) = state.pending.lock().unwrap().clone() else {
        return Err(update_failed("No update to install."));
    };
    let Some(_installing) = state.guard.try_start() else {
        return Err(updates::Failure {
            kind: updates::FailureKind::Busy,
            detail: "An update is already being installed.".into(),
        });
    };
    *state.owner.lock().unwrap() = Some(window.label().to_string());
    state.download.lock().unwrap().reset();
    let version = update.version.clone();
    let result = download_then_install(&state, update, on_progress).await;
    if result.is_ok() {
        *state.installed.lock().unwrap() = Some(version);
    }
    result
}

async fn download_then_install(
    state: &UpdateState,
    update: tauri_plugin_updater::Update,
    on_progress: Channel<UpdateProgress>,
) -> Result<(), updates::Failure> {
    let sender = on_progress.clone();
    let fetch = update.clone();
    let stop = DownloadStop::default();
    if let Some(cancelled) = state.download.lock().unwrap().started(stop.clone()) {
        cancelled.abort();
    }
    // On the blocking pool: `download` verifies the whole package's
    // signature when the bytes are in, which is long CPU work.
    let joined = blocking(move || {
        tauri::async_runtime::block_on(async move {
            let mut downloaded = 0u64;
            let download = fetch.download(
                |chunk, total| {
                    downloaded += chunk as u64;
                    let _ = sender.send(UpdateProgress { phase: "downloading", downloaded, total });
                },
                || {},
            );
            updates::until_stopped(download, &stop.0).await
        })
    })
    .await;
    // A Cancel that came after the bytes arrived must still stop the install.
    if !state.download.lock().unwrap().finished() {
        return Err(update_failed(progress::CANCELLED));
    }
    let bytes = downloaded_bytes(joined)?;
    let total = Some(bytes.len() as u64);
    let _ = on_progress.send(UpdateProgress { phase: "installing", downloaded: bytes.len() as u64, total });
    blocking(move || update.install(bytes))
        .await
        .map_err(|e| update_failed(&e))?
        .map_err(|e| updates::while_installing(failure_from(e)))
}

/// The download's result: its bytes, its error, or "Cancelled." when
/// `cancel_update` stopped it (`None`).
fn downloaded_bytes(
    joined: Result<Option<Result<Vec<u8>, tauri_plugin_updater::Error>>, String>,
) -> Result<Vec<u8>, updates::Failure> {
    match joined {
        Ok(Some(result)) => result.map_err(failure_from),
        Ok(None) => Err(update_failed(progress::CANCELLED)),
        Err(e) => Err(update_failed(&e)),
    }
}

/// Cancel the install unless installing already started; until then nothing
/// has been changed on disk. Only the window that started it can.
#[tauri::command]
fn cancel_update(window: tauri::Window, state: tauri::State<UpdateState>) {
    if state.owner.lock().unwrap().as_deref() != Some(window.label()) {
        return;
    }
    if let Some(download) = state.download.lock().unwrap().cancel() {
        download.abort();
    }
}

/// Whether a copy, move or other operation runs in any window: Restart Now
/// waits for them, since restarting would cut them off.
#[tauri::command]
fn operations_running(registry: tauri::State<progress::Registry>) -> bool {
    !registry.is_empty()
}

/// Start the installed new version, closing every window like Quit. Refused
/// while an operation runs in any window.
#[tauri::command]
fn restart_app(app: tauri::AppHandle, registry: tauri::State<progress::Registry>) -> Result<(), updates::Failure> {
    if !registry.is_empty() {
        return Err(update_failed("Operations are still running."));
    }
    app.restart()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let own_env = platform::webview_env_to_set(|k| std::env::var_os(k).is_some());
    for (key, value) in &own_env {
        // SAFETY: still single-threaded; Tauri and the webview start below.
        unsafe { std::env::set_var(key, value) };
    }
    // Ours, for our webview only: programs we start don't inherit them.
    platform::note_own_env(own_env.iter().map(|(key, _)| *key).collect());
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(UpdateState::default())
        .setup(|app| {
            let config_dir = share::identity::config_dir_override().unwrap_or_else(|| {
                app.path()
                    .app_config_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
            });
            let cache_dir = app
                .path()
                .app_cache_dir()
                .unwrap_or_else(|_| PathBuf::from("."));
            let home = app.path().home_dir().unwrap_or_else(|_| PathBuf::from("/"));
            app.manage(VersionsState::new(config_dir.join("versioned.json")));
            app.manage(progress::Registry::default());
            app.manage(thumbnails::Requests::default());
            app.manage(file_icons::Icons::new(Some(home.clone()), desktop_icon_theme()));
            app.manage(Searches::default());
            app.manage(share::file_share::FileShares::new(
                std::env::var("FILEBROWSER_FILE_PAGE").ok(),
                std::env::var("FILEBROWSER_RELAY").ok(),
            ));
            app.manage(CloudState::new(&config_dir));
            app.manage(sftp::SftpState::new(
                config_dir.join("sftp_known_hosts"),
                home,
                cache_dir.join("sftp"),
            ));
            // Drop copies of remote files nobody has touched for a week.
            let download_dirs: Vec<PathBuf> = downloads::DIRS
                .iter()
                .map(|dir| cache_dir.join(dir))
                .collect();
            std::thread::spawn(move || {
                let trash = |path: &std::path::Path| trash::delete(path).map_err(|e| e.to_string());
                for dir in &download_dirs {
                    downloads::prune(dir, std::time::SystemTime::now(), downloads::MAX_AGE, &trash);
                }
            });
            app.manage(share::state::ShareState::new(
                config_dir.clone(),
                cache_dir,
            ));
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
            search_dir,
            cancel_search,
            check_update,
            install_update,
            cancel_update,
            restart_app,
            operations_running,
            sftp_connect,
            sftp_servers,
            sftp_disconnect,
            sftp_open,
            extract_archive,
            compress_archive,
            new_window,
            window_buttons,
            window_control,
            list_apps,
            inspect_dmg,
            install_dmg,
            install_appimage,
            run_shell_script,
            open_file,
            open_url,
            open_with,
            preview_text,
            rename_many,
            set_permissions,
            create_symlink,
            create_file,
            list_templates,
            item_counts,
            make_dir,
            rename_entry,
            path_exists,
            path_info,
            file_properties,
            folder_size,
            new_folder_walk,
            cancel_folder_size,
            copy_entry,
            cancel_operation,
            measure,
            move_entry,
            trash_entries,
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
            cancel_thumbnails,
            icon_ids,
            app_icon_ids,
            file_share_prepare,
            file_share_status,
            file_share_stop,
            icon_image,
            volumes,
            drive_accounts,
            drive_disconnect,
            drive_client_status,
            drive_client_set,
            drive_client_clear,
            drive_connect_start,
            drive_connect_finish,
            drive_connect_cancel,
            drive_open,
            share_identity,
            set_share_name,
            share_folder,
            share_info,
            list_shares,
            create_link,
            share_link_requires_passphrase,
            share_link,
            remove_access,
            stop_sharing,
            add_received,
            list_received,
            remove_received,
            share_open,
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

#[cfg(test)]
mod update_tests {
    use super::*;
    use tauri_plugin_updater::Error as E;
    use updates::FailureKind;

    #[test]
    fn updater_errors_become_failure_kinds() {
        let kind = |e| failure_from(e).kind;
        let io = |k| E::Io(std::io::Error::from(k));
        assert_eq!(kind(io(std::io::ErrorKind::PermissionDenied)), FailureKind::NoPermission);
        assert_eq!(kind(io(std::io::ErrorKind::NotFound)), FailureKind::Failed);
        assert_eq!(kind(E::MissingSignedVersion), FailureKind::Unverified);
        let mismatch = E::SignedVersionMismatch { signed: "0.5.3".into(), announced: "0.6.0".into() };
        assert_eq!(kind(mismatch), FailureKind::Unverified);
        assert_eq!(kind(E::SignatureUtf8("x".into())), FailureKind::Unverified);
        assert_eq!(kind(E::Network("reset".into())), FailureKind::Failed);
        assert_eq!(kind(E::ReleaseNotFound), FailureKind::Failed);
        assert_eq!(
            failure_from(E::ReleaseNotFound).detail,
            "Could not fetch a valid release JSON from the remote"
        );
    }

    #[test]
    fn a_finished_download_hands_over_its_bytes() {
        assert_eq!(downloaded_bytes(Ok(Some(Ok(vec![1u8, 2, 3])))).unwrap(), vec![1, 2, 3]);
        let failed = downloaded_bytes(Ok(Some(Err(E::Network("reset".into()))))).unwrap_err();
        assert_eq!(failed.detail, "`reset`");
        let lost = downloaded_bytes(Err("Background task failed: panicked".into())).unwrap_err();
        assert_eq!((lost.kind, lost.detail.as_str()), (FailureKind::Failed, "Background task failed: panicked"));
    }

    #[test]
    fn a_download_can_run_on_the_blocking_pool() {
        // install_update's pattern: block_on inside blocking(…), which must
        // not panic as it would on an async worker.
        let stop = DownloadStop::default();
        let signal = stop.clone();
        let result = tauri::async_runtime::block_on(blocking(move || {
            tauri::async_runtime::block_on(updates::until_stopped(async { vec![1u8] }, &signal.0))
        }));
        assert_eq!(result, Ok(Some(vec![1u8])));
        stop.abort();
        let stopped = tauri::async_runtime::block_on(blocking(move || {
            tauri::async_runtime::block_on(updates::until_stopped(std::future::pending::<u8>(), &stop.0))
        }));
        assert_eq!(stopped, Ok(None));
    }

    #[tokio::test]
    async fn a_stopped_download_reports_cancelled() {
        let stop = DownloadStop::default();
        stop.abort();
        let stopped = updates::until_stopped(std::future::pending::<Result<Vec<u8>, E>>(), &stop.0).await;
        let failure = downloaded_bytes(Ok(stopped)).unwrap_err();
        assert_eq!((failure.kind, failure.detail.as_str()), (FailureKind::Failed, "Cancelled."));
    }
}

#[cfg(test)]
mod updater_config_tests {
    /// The shipped updater settings: a signed update must also name the
    /// version it signs (CI signatures do), and nothing may weaken transport.
    #[test]
    fn the_updater_requires_signed_versions_over_https() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let updater = &conf["plugins"]["updater"];
        assert_eq!(updater["requireSignedVersion"], true);
        assert!(updater["pubkey"].as_str().is_some_and(|key| !key.is_empty()));
        for endpoint in updater["endpoints"].as_array().unwrap() {
            assert!(endpoint.as_str().unwrap().starts_with("https://"));
        }
        assert!(updater.get("dangerousInsecureTransportProtocol").is_none());
        assert!(conf["bundle"].get("createUpdaterArtifacts").is_none(), "only CI turns this on");
    }
}

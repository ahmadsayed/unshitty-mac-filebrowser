mod archive;
mod cloud;
mod config_store;
mod downloads;
mod file_icons;
mod fs_ops;
mod place;
mod placement;
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
use cloud::service::OpenTarget;
use place::{Kind, Route};
use versions::history::History;
use versions::state::{Transfer, VersionedTrash, VersionsState};
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
        measure_paths(paths, |drive| cloud::transfer::measure(&app.state::<CloudState>(), drive))
    })
    .await?
}

/// The total of `paths`: Drive paths through `drive`, the others on disk.
fn measure_paths(
    paths: Vec<String>,
    drive: impl FnOnce(&[String]) -> Result<fs_ops::Measure, String>,
) -> Result<fs_ops::Measure, String> {
    let (drive_paths, others): (Vec<String>, Vec<String>) =
        paths.into_iter().partition(|path| place::kind(path) == Kind::Drive);
    let local: Vec<PathBuf> = others
        .iter()
        .filter_map(|path| place::local(path).ok().map(Path::to_path_buf))
        .collect();
    let mut total = fs_ops::measure(&local);
    if !drive_paths.is_empty() {
        let remote = drive(&drive_paths)?;
        total.bytes += remote.bytes;
        total.items += remote.items;
    }
    Ok(total)
}

#[tauri::command]
async fn list_dir(
    app: tauri::AppHandle,
    path: String,
    show_hidden: bool,
) -> Result<Vec<fs_ops::Entry>, String> {
    match place::kind(&path) {
        Kind::Share => {
            let state = app.state::<share::state::ShareState>();
            let hint = share_versions_hint(&app);
            state.list_dir(&path, show_hidden, hint).await
        }
        Kind::Sftp => {
            let app = app.clone();
            blocking(move || app.state::<sftp::SftpState>().list(&path, show_hidden)).await?
        }
        Kind::Drive => {
            blocking(move || {
                let state = app.state::<CloudState>();
                cloud::service::list_dir(&state, &path, show_hidden)
            })
            .await?
        }
        Kind::Local => list_local(path, show_hidden).await,
    }
}

/// `list_dir` for a folder on this computer.
async fn list_local(path: String, show_hidden: bool) -> Result<Vec<fs_ops::Entry>, String> {
    place::local(&path)?;
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
        search_local(
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

/// `search_dir` under a folder on this computer.
fn search_local(
    path: &str,
    query: &str,
    show_hidden: bool,
    limit: usize,
    found: &mut dyn FnMut(Vec<fs_ops::Entry>),
    cancelled: &dyn Fn() -> bool,
) -> Result<fs_ops::SearchSummary, String> {
    place::local(path)?;
    fs_ops::search_dir(path, query, show_hidden, limit, found, cancelled)
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
    place::local(&path)?;
    place::local_write(&dest)?;
    blocking(move || archive::extract_archive(path, dest, create)).await?
}

#[tauri::command]
async fn compress_archive(paths: Vec<String>, dest: String, format: String) -> Result<(), String> {
    place::all_local(&paths)?;
    place::local_write(&dest)?;
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

/// Drag local files out of the window to another app (see
/// `platform::begin_file_drag`); called while the button is still held.
/// Returns whether a drag started; `on_end` gets a message when it is over:
/// whether it was dropped on this window.
/// Linux only: macOS uses the drag plugin. A plain (sync) command, so it
/// runs on the main thread, where GTK must be called.
#[tauri::command]
fn start_file_drag(window: tauri::WebviewWindow, paths: Vec<String>, on_end: Channel<bool>) -> Result<bool, String> {
    let uris = platform::drag_uris(&paths)?;
    #[cfg(target_os = "linux")]
    {
        let gtk_window = window.gtk_window().map_err(|e| e.to_string())?;
        platform::begin_file_drag(&gtk_window, uris, move |landed_here| {
            let _ = on_end.send(landed_here);
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (window, uris, on_end);
        Err("Dragging out uses the drag plugin on this platform.".into())
    }
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

/// What opening `path` opens: the file itself on this computer; for Drive,
/// a share or an SSH server, a copy downloaded into the app cache, or a
/// Google Docs file's web page. A download is operation `op`: it reports
/// through `on_progress` and `cancel_operation` stops it.
#[tauri::command]
async fn open_target(
    app: tauri::AppHandle,
    path: String,
    op: String,
    on_progress: Channel<ProgressUpdate>,
) -> Result<OpenTarget, String> {
    if place::kind(&path) == Kind::Local {
        return open_local_target(path);
    }
    let tracking = Tracking::start(&app, &op, on_progress);
    let result = download_to_open(&app, path, &tracking.progress).await;
    tracking.finish(&app);
    result
}

/// `open_target` for a path on this computer: the file itself.
fn open_local_target(path: String) -> Result<OpenTarget, String> {
    place::local(&path)?;
    Ok(OpenTarget::File { path })
}

/// A remote file's copy to open, downloaded by its kind's handler (each
/// parses its own scheme and refuses a malformed path).
async fn download_to_open(
    app: &tauri::AppHandle,
    path: String,
    progress: &progress::Progress,
) -> Result<OpenTarget, String> {
    match place::kind(&path) {
        Kind::Drive => drive_target(app.clone(), path, progress.clone()).await,
        Kind::Share => share_target(app, &path, progress).await,
        Kind::Sftp => sftp_target(app.clone(), path, progress.clone()).await,
        Kind::Local => open_local_target(path),
    }
}

/// A Drive file's snapshot in the app cache, or a Google Docs file's page.
async fn drive_target(
    app: tauri::AppHandle,
    path: String,
    progress: progress::Progress,
) -> Result<OpenTarget, String> {
    blocking(move || {
        let cache = app.path().app_cache_dir().map_err(|error| error.to_string())?;
        cloud::service::open(&app.state::<CloudState>(), &path, &cache, &progress)
    })
    .await?
}

/// A received share's file, fetched into the app cache.
async fn share_target(
    app: &tauri::AppHandle,
    path: &str,
    progress: &progress::Progress,
) -> Result<OpenTarget, String> {
    let state = app.state::<share::state::ShareState>();
    let path = state.open(path, share_versions_hint(app), progress).await?;
    Ok(OpenTarget::File { path })
}

/// An SSH server's file, downloaded into the app cache.
async fn sftp_target(
    app: tauri::AppHandle,
    path: String,
    progress: progress::Progress,
) -> Result<OpenTarget, String> {
    let path = blocking(move || app.state::<sftp::SftpState>().open(&path, &progress)).await??;
    Ok(OpenTarget::File { path })
}

#[tauri::command]
async fn open_file(path: String) -> Result<(), String> {
    place::local(&path)?;
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
    place::local(&path)?;
    blocking(move || platform::open_with(&path, &app)).await?
}

#[tauri::command]
async fn preview_text(path: String, limit: usize) -> Result<fs_ops::TextPreview, String> {
    place::local(&path)?;
    blocking(move || fs_ops::preview_text(path, limit)).await?
}

/// A local file's bytes for the 3D preview, at most 512 MiB. Raw bytes
/// avoid JSON-encoding them.
#[tauri::command]
async fn preview_bytes(path: String, limit: u64) -> Result<tauri::ipc::Response, String> {
    place::local(&path)?;
    let limit = limit.clamp(1, 512 * 1024 * 1024);
    let bytes = blocking(move || fs_ops::preview_bytes(path, limit)).await??;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
async fn rename_many(items: Vec<fs_ops::RenameItem>) -> Result<(), String> {
    for item in &items {
        place::local_write(&item.from)?;
        place::local_write(&item.to)?;
    }
    blocking(move || fs_ops::rename_many(items)).await?
}

#[tauri::command]
async fn set_permissions(path: String, mode: u32) -> Result<(), String> {
    place::local_write(&path)?;
    blocking(move || fs_ops::set_permissions(path, mode)).await?
}

#[tauri::command]
async fn create_symlink(path: String) -> Result<String, String> {
    place::local_write(&path)?;
    blocking(move || fs_ops::create_link(path)).await?
}

#[tauri::command]
async fn create_file(path: String) -> Result<(), String> {
    place::local_write(&path)?;
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
        // Remote and relative paths have no summary.
        Ok(place::local_only(&paths, |local| fs_ops::item_counts(local, show_hidden)))
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
        Ok(path_infos(&paths, |path| cloud::service::path_info(&state, path).ok()))
    })
    .await?
}

/// `path_info` for a batch: Drive paths through `drive`, shares and SSH
/// servers have none.
fn path_infos(
    paths: &[String],
    drive: impl Fn(&str) -> Option<fs_ops::PathInfo>,
) -> Vec<Option<fs_ops::PathInfo>> {
    let mut out = place::local_only(paths, fs_ops::path_info);
    for (info, path) in out.iter_mut().zip(paths) {
        if place::kind(path) == Kind::Drive {
            *info = drive(path);
        }
    }
    out
}

/// Why the Properties dialog can't describe `path` yet, if it can't.
fn properties_refusal(path: &str) -> Result<(), String> {
    match place::kind(path) {
        Kind::Sftp => Err("Properties aren't available for SSH servers yet.".into()),
        Kind::Share => Err("Properties aren't available for shared folders yet.".into()),
        Kind::Drive => Ok(()),
        Kind::Local => place::local(path).map(|_| ()),
    }
}

/// Size, dates, permissions and contents for the Properties dialog.
#[tauri::command]
async fn file_properties(
    app: tauri::AppHandle,
    path: String,
    show_hidden: bool,
) -> Result<properties::FileProperties, String> {
    properties_refusal(&path)?;
    if place::kind(&path) == Kind::Drive {
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
    folder_size_refusal(&path)?;
    blocking(move || properties::folder_size(&path, || properties::walk_cancelled(walk))).await?
}

/// Why `folder_size` can't measure `path`, if it can't: only folders on
/// this computer are walked.
fn folder_size_refusal(path: &str) -> Result<(), String> {
    match place::kind(path) {
        Kind::Sftp => Err("Folder sizes aren't available for SSH servers yet.".into()),
        Kind::Share => Err("Folder sizes aren't available for shared folders yet.".into()),
        Kind::Drive => Err(cloud::service::unsupported()),
        Kind::Local => place::local(path).map(|_| ()),
    }
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
    if place::kind(&path) == Kind::Drive {
        return blocking(move || cloud::write::make_dir(&app.state::<CloudState>(), &path)).await?;
    }
    make_local_dir(path)
}

/// `make_dir` anywhere but Drive: refused for shares, SSH servers and
/// anything that isn't a path on this computer.
fn make_local_dir(path: String) -> Result<String, String> {
    place::local_write(&path)?;
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
        let versions = app.state::<VersionsState>();
        let trash_local = VersionedTrash::new(&versions, AppTrash { app: &app });
        let state = app.state::<CloudState>();
        cloud::transfer::dispatch(
            &state,
            &from,
            &to,
            overwrite,
            moving,
            &progress,
            &trash_local,
        )
    })
    .await?
}

#[tauri::command]
async fn path_exists(app: tauri::AppHandle, path: String) -> bool {
    blocking(move || {
        exists_at(&path, |path| cloud::service::path_exists(&app.state::<CloudState>(), path))
    })
    .await
    .unwrap_or(false)
}

/// `path_exists`: Drive paths through `drive`, the others on disk.
fn exists_at(path: &str, drive: impl FnOnce(&str) -> bool) -> bool {
    match place::kind(path) {
        Kind::Drive => drive(path),
        _ => place::local(path).is_ok_and(|path| fs_ops::path_exists(path.to_string_lossy().into())),
    }
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
    let route = place::transfer(&from, &to)?;
    let tracking = Tracking::start(&app, &op, on_progress);
    let progress = tracking.progress.clone();
    let result = if route == Route::Drive {
        drive_transfer(app.clone(), from, to, overwrite, false, progress).await
    } else {
        let app = app.clone();
        blocking(move || {
            let state = app.state::<VersionsState>();
            let (from_path, to_path) = (PathBuf::from(&from), PathBuf::from(&to));
            state.transfer(&from_path, &to_path, Transfer::Copy, || {
                let trash = AppTrash { app: &app };
                fs_ops::copy_entry_with(from, to.clone(), overwrite, &progress, &trash)
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
    let route = place::transfer(&from, &to)?;
    let tracking = Tracking::start(&app, &op, on_progress);
    let progress = tracking.progress.clone();
    let result = if route == Route::Drive {
        drive_transfer(app.clone(), from, to, overwrite, true, progress).await
    } else {
        let app = app.clone();
        blocking(move || {
            let state = app.state::<VersionsState>();
            let (from_path, to_path) = (PathBuf::from(&from), PathBuf::from(&to));
            state.transfer(&from_path, &to_path, Transfer::Move, || {
                let trash = AppTrash { app: &app };
                fs_ops::move_entry_with(from, to.clone(), overwrite, &progress, &trash)
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
    // write::rename refuses a local path on either side.
    if place::transfer(&from, &to)? == Route::Drive {
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

/// The home Trash and this app's log of it (`trash_ops::Trash`). Missing
/// only when the home folder is unknown, which every Trash command reports.
fn trash_of(app: &tauri::AppHandle) -> Result<tauri::State<'_, trash_ops::Trash>, String> {
    app.try_state::<trash_ops::Trash>()
        .ok_or_else(|| "The home folder is unknown".to_string())
}

/// Send `path` to the Trash through the app's log (`Trash::send`), so
/// Restore From Trash knows where it came from.
fn send_to_trash(app: &tauri::AppHandle, path: &Path) -> Result<(), String> {
    trash_of(app)?.send(path).map(|_| ())
}

/// The app's logged Trash as a copy, a move or a Drive transfer uses it
/// (`placement::ReplaceTrash`): Restore knows where each item came from,
/// and the old item a failed Replace takes back is forgotten. Looked up only
/// when something goes to the Trash. A local copy or move closes the
/// histories inside an item with `VersionsState::transfer`; a Drive
/// transfer wraps this in `VersionedTrash`, which closes them before an
/// item goes and reopens them if a failed Replace takes it back.
struct AppTrash<'a> {
    app: &'a tauri::AppHandle,
}

impl placement::ReplaceTrash for AppTrash<'_> {
    fn send(&self, path: &Path) -> Result<Option<PathBuf>, String> {
        trash_of(self.app)?.send(path)
    }

    fn take_back(&self, trashed: &Path, origin: &Path) -> Result<(), String> {
        trash_of(self.app)?.untrash(trashed, origin)
    }
}

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
    if let Ok(trash) = trash_of(app) {
        trash.record(&moves);
    }
    results.into_iter().map(TrashOutcome::from).collect()
}

/// A path of a Move to Trash that doesn't go to a local Trash: sent to
/// Drive's own Trash (D10; never a permanent delete), or refused
/// (`local_trash_refusal`). `None` for local paths.
fn trash_elsewhere(
    app: &tauri::AppHandle,
    path: &str,
    progress: &progress::Progress,
) -> Option<Result<Option<String>, String>> {
    if place::kind(path) != Kind::Drive {
        return local_trash_refusal(path);
    }
    let result = progress
        .check()
        .and_then(|()| cloud::write::trash(&app.state::<CloudState>(), path));
    if result.is_ok() {
        progress.add_item();
    }
    Some(result)
}

/// Why a path outside Drive can't go to the local Trash, if it can't:
/// shares and SSH servers are read only, and anything else must be a path
/// on this computer.
fn local_trash_refusal(path: &str) -> Option<Result<Option<String>, String>> {
    place::local_write(path).err().map(Err)
}

impl From<Result<Option<String>, String>> for TrashOutcome {
    fn from(result: Result<Option<String>, String>) -> Self {
        match result {
            Ok(trashed) => TrashOutcome { trashed, error: None },
            Err(error) => TrashOutcome { trashed: None, error: Some(error) },
        }
    }
}

/// Original location and trash date of items in the Trash; nothing for a
/// path that isn't on this computer.
#[tauri::command]
async fn trash_info(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<Vec<Option<trash_ops::TrashInfo>>, String> {
    blocking(move || {
        trash_of(&app)
            .map(|trash| trash_infos(&trash, &paths))
            .unwrap_or_default()
    })
    .await
}

/// `trash_info` with the home Trash.
fn trash_infos(trash: &trash_ops::Trash, paths: &[String]) -> Vec<Option<trash_ops::TrashInfo>> {
    place::local_only(paths, |local| trash.info(&local).into_iter().map(Some).collect())
}

/// "Restore From Trash" and Undo: move an item back where it came from.
/// Returns the restored path.
#[tauri::command]
async fn restore_entry(app: tauri::AppHandle, path: String) -> Result<String, String> {
    if place::kind(&path) == Kind::Drive {
        return blocking(move || cloud::write::restore(&app.state::<CloudState>(), &path)).await?;
    }
    let trashed = place::local(&path)?.to_path_buf();
    blocking(move || {
        let origin = trash_of(&app)?.restore(&trashed)?;
        Ok(origin.to_string_lossy().to_string())
    })
    .await?
}

/// Delete an item in the Trash for good. Not for Drive's own Trash yet:
/// `place::local` refuses it, as every other path that isn't on this computer.
#[tauri::command]
async fn delete_permanently(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let path = place::local(&path)?.to_path_buf();
    blocking(move || trash_of(&app)?.delete_permanently(&path)).await?
}

/// The folder the Trash view lists. On Linux this is
/// `~/.local/share/Trash/files`, not `~/.Trash`, and is created if missing.
#[tauri::command]
fn trash_dir(app: tauri::AppHandle) -> String {
    trash_of(&app)
        .map(|trash| trash.listing_root().to_string_lossy().to_string())
        .unwrap_or_default()
}

/// True when `path` is inside the user's home Trash or its volume's Trash
/// (`Trash::contains`); versioned_root, shared folders and the program and
/// disk-image actions refuse it.
fn in_trash(app: &tauri::AppHandle, path: &Path) -> bool {
    trash_of(app).is_ok_and(|trash| trash.contains(path))
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
            versions.trash(old, || send_to_trash(&app, old))
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
    blocking(move || make_versioned(&app.state::<VersionsState>(), path)).await?
}

/// `make_versioned_dir` with `state`, on this computer only.
fn make_versioned(state: &VersionsState, path: String) -> Result<(), String> {
    let root = place::local_write(&path)?.to_path_buf();
    fs_ops::make_dir(path)?;
    match History::create(&root) {
        Ok(history) => {
            state.adopt(history);
            Ok(())
        }
        Err(error) => Err(format!(
            "The folder was created, but versions could not be turned on: {error}"
        )),
    }
}

/// The nearest versioned root for a location, if any. Cheap: it reads
/// `.versions/meta.json`, not SQLite.
#[tauri::command]
async fn versioned_root(app: tauri::AppHandle, path: String) -> Option<String> {
    let path = place::local(&path).ok()?.to_path_buf();
    blocking(move || {
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
    history_in(&app.state::<VersionsState>(), path)
}

/// The history keeping versions of `path`, a path on this computer, and
/// `path` relative to its root.
fn history_in(
    state: &VersionsState,
    path: &str,
) -> Result<(std::sync::Arc<History>, PathBuf), String> {
    let path = place::local(path)?;
    if versions::is_internal_path(path) {
        return Err("This folder does not keep versions.".to_string());
    }
    let Some((history, rel)) = state
        .history_for(path)
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
        restore::replace_with_version(&history, &rel, version, |old| send_to_trash(&app, old))
            .map_err(|e| e.to_string())
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
    blocking(move || history_info_in(&app.state::<VersionsState>(), &root)).await?
}

/// `history_info` with `state`.
fn history_info_in(state: &VersionsState, root: &str) -> Result<versions::HistoryInfo, String> {
    state.info_under(place::local(root)?).map_err(|e| e.to_string())
}

#[tauri::command]
async fn clear_history(app: tauri::AppHandle, root: String) -> Result<(), String> {
    blocking(move || {
        clear_history_in(&app.state::<VersionsState>(), &root, |dir| trash_history_dir(&app, dir))
    })
    .await?
}

/// `clear_history` with `state`, sending the old `.versions/` away with `trash_dir`.
fn clear_history_in(
    state: &VersionsState,
    root: &str,
    trash_dir: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    state.clear(place::local(root)?, trash_dir)
}

#[tauri::command]
async fn stop_keeping_versions(app: tauri::AppHandle, root: String) -> Result<(), String> {
    blocking(move || {
        stop_keeping_in(&app.state::<VersionsState>(), &root, |dir| trash_history_dir(&app, dir))
    })
    .await?
}

/// `stop_keeping_versions` with `state`, sending `.versions/` away with `trash_dir`.
fn stop_keeping_in(
    state: &VersionsState,
    root: &str,
    trash_dir: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    state.stop_keeping(place::local(root)?, trash_dir)
}

/// Clear History and Stop Keeping Versions move `.versions/` to the Trash
/// (D7), logged like any other item so Restore can put it back.
fn trash_history_dir(app: &tauri::AppHandle, dir: &Path) -> Result<(), String> {
    send_to_trash(app, dir)
        .map_err(|e| format!("Could not move the version history to the Trash: {e}"))
}

#[tauri::command]
async fn empty_trash(
    app: tauri::AppHandle,
    op: String,
    on_progress: Channel<ProgressUpdate>,
) -> Result<(), String> {
    let tracking = Tracking::start(&app, &op, on_progress);
    let progress = tracking.progress.clone();
    let worker = app.clone();
    let result = blocking(move || trash_of(&worker)?.empty(&progress)).await;
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
    let png = if place::kind(&path) == Kind::Drive {
        blocking(move || {
            let state = app.state::<CloudState>();
            cloud::service::thumbnail(&state, &path, size)
        })
        .await
        .unwrap_or_default()
    } else if place::local(&path).is_err() {
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

/// Start sharing local files with a QR code; the id names the share for
/// `file_share_status` and `file_share_stop`. See `share::file_share`.
#[tauri::command]
async fn file_share_prepare(
    window: tauri::WebviewWindow,
    shares: tauri::State<'_, share::file_share::FileShares>,
    paths: Vec<String>,
) -> Result<String, String> {
    shares.prepare(&qr_sources(&paths)?, window.label())
}

/// What Send with QR Code serves: files on this computer only.
fn qr_sources(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    place::all_local(paths).map_err(|_| "Only files on this computer can be sent with a QR code.".into())
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
        os_icon_ids(&paths, |path| icons.id(path))
    })
    .await
    .unwrap_or_default()
}

/// `icon_ids` for a batch: `id` is asked about local paths only.
fn os_icon_ids(paths: &[String], id: impl Fn(&Path) -> Option<String>) -> Vec<Option<String>> {
    place::local_only(paths, |local| local.iter().map(|path| id(Path::new(path))).collect())
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
    account: Option<String>,
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
            let about = cloud::service::about(&state, &token.access_token)?;
            // Signing in for an account that lost its session keeps its id.
            state.record_sign_in(about, &token, account.as_deref())
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
    let path = share_source(&path)?;
    let hint = share_versions_hint(&app);
    let trash = app.clone();
    state
        .share_folder(path, move |p| in_trash(&trash, p), hint)
        .await
}

/// The folder a share command names: a folder on this computer, refused
/// before the sharing node starts. `share_info` has nothing for another path.
fn share_source(path: &str) -> Result<&Path, String> {
    place::local(path).map_err(|_| "Only folders on this computer can be shared.".into())
}

#[tauri::command]
async fn share_info(
    app: tauri::AppHandle,
    state: tauri::State<'_, share::state::ShareState>,
    path: String,
) -> Result<Option<share::state::ShareInfo>, String> {
    let Ok(path) = share_source(&path) else {
        return Ok(None);
    };
    let hint = share_versions_hint(&app);
    state.info(path, hint).await
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
        // A closed window's QR shares stop with it, even if its page had no
        // time to say so.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(shares) = window.try_state::<share::file_share::FileShares>() {
                    shares.stop_window(window.label());
                }
            }
        })
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
            if let Ok(home) = app.path().home_dir() {
                let log = config_file(app.handle(), "trash_log.json").ok();
                app.manage(trash_ops::Trash::new(home, log));
            }
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
            let pruning = app.handle().clone();
            std::thread::spawn(move || {
                // Edited copies go to the Trash logged, like any other item.
                let trash = |path: &std::path::Path| send_to_trash(&pruning, path);
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
            open_target,
            extract_archive,
            compress_archive,
            new_window,
            window_buttons,
            window_control,
            start_file_drag,
            list_apps,
            inspect_dmg,
            install_dmg,
            install_appimage,
            run_shell_script,
            open_file,
            open_url,
            open_with,
            preview_text,
            preview_bytes,
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
mod properties_tests {
    use super::properties_refusal;

    #[test]
    fn properties_are_refused_for_shared_folders_and_ssh_servers() {
        assert_eq!(
            properties_refusal("share://r1/hello.txt").unwrap_err(),
            "Properties aren't available for shared folders yet."
        );
        assert_eq!(
            properties_refusal("sftp://me@host/home").unwrap_err(),
            "Properties aren't available for SSH servers yet."
        );
        assert_eq!(properties_refusal("/home/me/a.txt"), Ok(()));
        assert_eq!(properties_refusal("gdrive://acct/root~My%20Drive"), Ok(()));
        assert_eq!(properties_refusal("a.txt").unwrap_err(), "“a.txt” is not a local file.");
    }
}

#[cfg(test)]
mod dispatch_tests {
    //! What the commands that tell local paths from remote ones answer for
    //! each kind of path, malformed remote paths included.
    use super::*;
    use std::cell::RefCell;

    const DRIVE: &str = "gdrive://acct/root~My%20Drive";
    const BAD_DRIVE: &str = "gdrive://";
    const SHARE: &str = "share://r1/Photos";
    const BAD_SHARE: &str = "share://";
    const SFTP: &str = "sftp://me@host/home";
    const BAD_SFTP: &str = "sftp://";
    /// A folder relative to the tests' working directory (the crate root).
    const RELATIVE: &str = "src";

    struct Fixture {
        _dir: tempfile::TempDir,
        folder: String,
        file: String,
    }

    /// `folder` holds one 3-byte file; `file` is 5 bytes.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("a.txt"), "abc").unwrap();
        let file = dir.path().join("file.txt");
        std::fs::write(&file, "hello").unwrap();
        let text = |p: PathBuf| p.to_string_lossy().to_string();
        Fixture { folder: text(folder), file: text(file), _dir: dir }
    }

    fn remote() -> Vec<String> {
        [DRIVE, BAD_DRIVE, SHARE, BAD_SHARE, SFTP, BAD_SFTP].map(String::from).to_vec()
    }

    fn strings(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn item_counts_summarises_local_folders_only() {
        use tauri::async_runtime::block_on;
        let counts = |paths: Vec<String>| block_on(item_counts(paths, false)).unwrap();
        let f = fixture();
        let mut paths = vec![f.folder.clone(), f.file.clone()];
        paths.extend(remote());
        let out = counts(paths.clone());
        assert_eq!(out.len(), paths.len());
        assert_eq!(out[0], Some(fs_ops::FolderSummary { count: 1, versioned: false }));
        assert!(out[1..].iter().all(Option::is_none), "{out:?}");
        assert_eq!(counts(strings(&[RELATIVE])), vec![None]);
    }

    #[test]
    fn list_dir_lists_local_folders_and_refuses_relative_ones() {
        // list_dir sends the remote kinds to their own handlers; the rest
        // come here. `src` exists in the tests' working directory.
        use tauri::async_runtime::block_on;
        let f = fixture();
        let listed = block_on(list_local(f.folder.clone(), false)).unwrap();
        assert_eq!(listed.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a.txt"]);
        assert_eq!(block_on(list_local(RELATIVE.into(), false)).unwrap_err(), "“src” is not a local file.");
        assert_eq!(block_on(list_local(String::new(), false)).unwrap_err(), "“” is not a local file.");
    }

    #[test]
    fn path_info_asks_drive_and_has_nothing_for_shares_and_ssh() {
        let f = fixture();
        let mut paths = vec![f.folder.clone(), f.file.clone()];
        paths.extend(remote());
        let asked = RefCell::new(Vec::new());
        let out = path_infos(&paths, |path| {
            asked.borrow_mut().push(path.to_string());
            Some(fs_ops::PathInfo { is_dir: true, device: 7 })
        });
        assert_eq!(asked.into_inner(), strings(&[DRIVE, BAD_DRIVE]));
        let kinds: Vec<Option<(bool, bool)>> =
            out.iter().map(|o| o.as_ref().map(|i| (i.is_dir, i.device == 7))).collect();
        assert_eq!(
            kinds,
            vec![Some((true, false)), Some((false, false)), Some((true, true)), Some((true, true)), None, None, None, None]
        );
        assert_eq!(path_infos(&strings(&[RELATIVE]), |_| panic!("not Drive")), vec![None]);
    }

    #[test]
    fn path_exists_asks_drive_and_looks_on_disk_for_the_rest() {
        let f = fixture();
        assert!(exists_at(&f.file, |_| panic!("not Drive")));
        assert!(!exists_at(&format!("{}/gone", f.folder), |_| panic!("not Drive")));
        assert!(exists_at(DRIVE, |path| path == DRIVE));
        assert!(!exists_at(BAD_DRIVE, |_| false));
        for path in [SHARE, BAD_SHARE, SFTP, BAD_SFTP] {
            assert!(!exists_at(path, |_| panic!("not Drive")), "{path}");
        }
        assert!(!exists_at(RELATIVE, |_| panic!("not Drive")));
    }

    #[test]
    fn qr_share_refuses_every_remote_path() {
        let f = fixture();
        assert_eq!(qr_sources(&[f.file.clone()]).unwrap(), vec![PathBuf::from(&f.file)]);
        for path in remote() {
            assert_eq!(
                qr_sources(&[f.file.clone(), path.clone()]).unwrap_err(),
                "Only files on this computer can be sent with a QR code.",
                "{path}"
            );
        }
        assert!(qr_sources(&strings(&[RELATIVE])).is_err());
    }

    #[test]
    fn icon_ids_asks_the_os_about_local_paths_only() {
        let f = fixture();
        let mut paths = vec![f.file.clone()];
        paths.extend(remote());
        let asked = RefCell::new(Vec::new());
        let out = os_icon_ids(&paths, |path| {
            asked.borrow_mut().push(path.to_path_buf());
            Some("id".into())
        });
        assert_eq!(asked.into_inner(), vec![PathBuf::from(&f.file)]);
        assert_eq!(out[0].as_deref(), Some("id"));
        assert!(out[1..].iter().all(Option::is_none));
        assert_eq!(os_icon_ids(&strings(&[RELATIVE]), |_| panic!("not local")), vec![None]);
    }

    #[test]
    fn measure_sends_drive_paths_to_drive_and_counts_the_rest_on_disk() {
        let f = fixture();
        let on_disk = fs_ops::measure(&[PathBuf::from(&f.file), PathBuf::from(&f.folder)]);
        let mut paths = vec![f.file.clone(), f.folder.clone()];
        paths.extend(remote());
        let asked = RefCell::new(Vec::new());
        let total = measure_paths(paths, |drive| {
            asked.borrow_mut().extend_from_slice(drive);
            Ok(fs_ops::Measure { bytes: 100, items: 1 })
        })
        .unwrap();
        assert_eq!(asked.into_inner(), strings(&[DRIVE, BAD_DRIVE]));
        assert_eq!(total, fs_ops::Measure { bytes: on_disk.bytes + 100, items: on_disk.items + 1 });
        let failed = measure_paths(strings(&[DRIVE]), |_| Err("offline".into()));
        assert_eq!(failed.unwrap_err(), "offline");
        let relative = measure_paths(strings(&[RELATIVE]), |_| panic!("not Drive")).unwrap();
        assert_eq!(relative, fs_ops::Measure::default());
    }

    #[test]
    fn folder_size_walks_local_folders_only() {
        let f = fixture();
        assert_eq!(folder_size_refusal(&f.folder), Ok(()));
        for (path, refusal) in [
            (DRIVE, "This action is not available for Google Drive items yet."),
            (BAD_DRIVE, "This action is not available for Google Drive items yet."),
            (SHARE, "Folder sizes aren't available for shared folders yet."),
            (BAD_SHARE, "Folder sizes aren't available for shared folders yet."),
            (SFTP, "Folder sizes aren't available for SSH servers yet."),
            (BAD_SFTP, "Folder sizes aren't available for SSH servers yet."),
            (RELATIVE, "“src” is not a local file."),
        ] {
            assert_eq!(folder_size_refusal(path).unwrap_err(), refusal, "{path}");
        }
    }
}

#[cfg(test)]
mod write_guard_tests {
    //! The write commands that work on this computer only refuse every
    //! other path before anything is touched (the UI already offers them
    //! only there; this is the backend's own check).
    use super::*;
    use tauri::async_runtime::block_on;

    const SHARED: &str = "Shared folders are read only.";
    const SSH: &str = "SSH servers are read only for now.";
    const DRIVE: &str = "This action is not available for Google Drive items yet.";

    /// Each refused path with its refusal.
    fn refused() -> Vec<(&'static str, String)> {
        vec![
            ("share://r1/a.txt", SHARED.into()),
            ("share://", SHARED.into()),
            ("sftp://me@host/a.txt", SSH.into()),
            ("gdrive://acct/root~x/~a.txt", DRIVE.into()),
            ("gdrive://", DRIVE.into()),
            ("a.txt", "“a.txt” is not a local file.".into()),
        ]
    }

    fn local(dir: &tempfile::TempDir, name: &str) -> String {
        dir.path().join(name).to_string_lossy().to_string()
    }

    #[test]
    fn create_file_and_permissions_and_links_refuse_other_paths() {
        for (path, refusal) in refused() {
            assert_eq!(block_on(create_file(path.into())).unwrap_err(), refusal, "{path}");
            assert_eq!(block_on(set_permissions(path.into(), 0o644)).unwrap_err(), refusal, "{path}");
            assert_eq!(block_on(create_symlink(path.into())).unwrap_err(), refusal, "{path}");
        }
        assert!(!Path::new("a.txt").exists());
        let dir = tempfile::tempdir().unwrap();
        let file = local(&dir, "a.txt");
        block_on(create_file(file.clone())).unwrap();
        block_on(set_permissions(file.clone(), 0o600)).unwrap();
        assert_eq!(block_on(create_symlink(file)).unwrap(), local(&dir, "Link to a.txt"));
    }

    #[test]
    fn new_folder_on_this_computer_refuses_other_paths() {
        // make_dir sends Drive paths to Drive first; the rest come here.
        for (path, refusal) in refused() {
            assert_eq!(make_local_dir(path.into()).unwrap_err(), refusal, "{path}");
        }
        assert!(!Path::new("a.txt").exists());
        let dir = tempfile::tempdir().unwrap();
        let folder = local(&dir, "New Folder");
        assert_eq!(make_local_dir(folder.clone()), Ok(folder.clone()));
        assert!(Path::new(&folder).is_dir());
    }

    #[test]
    fn move_to_trash_hands_the_local_trash_absolute_paths_only() {
        // trash_elsewhere sends Drive paths to Drive's Trash first; the rest
        // are refused here or go to the local Trash.
        let mut paths = vec!["/Users/me/a.txt".to_string()];
        let others: Vec<(&str, String)> = refused().into_iter().filter(|(p, _)| place::kind(p) != Kind::Drive).collect();
        paths.extend(others.iter().map(|(path, _)| path.to_string()));
        paths.push("src".into());
        let (results, moves) = trash_ops::trash_routed(&paths, local_trash_refusal, |origins| {
            assert_eq!(origins, [PathBuf::from("/Users/me/a.txt")]);
            vec![Ok(None)]
        });
        assert!(moves.is_empty());
        let errors: Vec<Option<String>> = results.into_iter().map(Result::err).collect();
        let mut expected = vec![None];
        expected.extend(others.into_iter().map(|(_, refusal)| Some(refusal)));
        expected.push(Some("“src” is not a local file.".into()));
        assert_eq!(errors, expected);
        assert!(Path::new("src").is_dir());
    }

    #[test]
    fn rename_many_refuses_any_other_path_on_either_side() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (local(&dir, "a.txt"), local(&dir, "b.txt"));
        std::fs::write(&a, "a").unwrap();
        let item = |from: &str, to: &str| fs_ops::RenameItem { from: from.into(), to: to.into() };
        for (path, refusal) in refused() {
            let ok = item(&a, &b);
            assert_eq!(block_on(rename_many(vec![ok, item(path, &b)])).unwrap_err(), refusal, "{path}");
            assert_eq!(block_on(rename_many(vec![item(&a, path)])).unwrap_err(), refusal, "{path}");
            assert!(Path::new(&a).exists() && !Path::new(&b).exists(), "{path}");
        }
        block_on(rename_many(vec![item(&a, &b)])).unwrap();
        assert!(Path::new(&b).exists());
    }

    #[test]
    fn archives_need_local_sources_and_a_writable_local_destination() {
        let dir = tempfile::tempdir().unwrap();
        let file = local(&dir, "a.txt");
        std::fs::write(&file, "a").unwrap();
        let zip = local(&dir, "a.zip");
        for (path, refusal) in refused() {
            let to_remote = block_on(compress_archive(vec![file.clone()], path.into(), "zip".into()));
            assert_eq!(to_remote.unwrap_err(), refusal, "{path}");
            let extract_to = block_on(extract_archive(zip.clone(), path.into(), true));
            assert_eq!(extract_to.unwrap_err(), refusal, "{path}");
        }
        for (path, refusal) in [
            ("share://r1/a.zip", "This action is not available for shared folders."),
            ("sftp://me@host/a.zip", "This action is not available for SSH servers."),
            ("gdrive://acct/root~x", DRIVE),
            ("a.zip", "“a.zip” is not a local file."),
        ] {
            let from_remote = block_on(compress_archive(vec![file.clone(), path.into()], zip.clone(), "zip".into()));
            assert_eq!(from_remote.unwrap_err(), refusal, "{path}");
            let extract_from = block_on(extract_archive(path.into(), local(&dir, "out"), true));
            assert_eq!(extract_from.unwrap_err(), refusal, "{path}");
        }
        assert!(!Path::new(&zip).exists());
        block_on(compress_archive(vec![file], zip.clone(), "zip".into())).unwrap();
        let out = block_on(extract_archive(zip, local(&dir, "out"), true)).unwrap();
        assert_eq!(out.files, 1);
    }
}

/// The frontend folds names too (`src/search.ts`), to filter the folder
/// before the recursive search answers. Both run these cases, so the two
/// copies can't drift. Kept out of `fs_ops.rs` so that file still builds on
/// its own in a scratch crate.
#[cfg(test)]
mod fold_tests {
    use crate::fs_ops::fold;

    #[test]
    fn fold_agrees_with_the_frontend() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../tests/fold-cases.json")).unwrap();
        assert!(cases.len() >= 10);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let why = case["why"].as_str().unwrap();
            assert_eq!(fold(name), case["folded"].as_str().unwrap(), "{why}");
        }
    }
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

#[cfg(test)]
mod read_gate_tests {
    //! Search, the text preview and opening a file work on this computer
    //! only: every other path is refused before anything is read or launched.
    use super::*;
    use tauri::async_runtime::block_on;

    /// Each refused path with its refusal, with `relative` as the relative one.
    fn refused(relative: &'static str) -> Vec<(&'static str, String)> {
        vec![
            ("share://r1/a.txt", "This action is not available for shared folders.".into()),
            ("sftp://me@host/a.txt", "This action is not available for SSH servers.".into()),
            ("gdrive://acct/root~x/~a.txt", "This action is not available for Google Drive items yet.".into()),
            (relative, format!("“{relative}” is not a local file.")),
        ]
    }

    #[test]
    fn preview_and_search_read_local_paths_only() {
        // `Cargo.toml` and `src` are in the tests' working directory (the
        // crate root), so their refusal isn't a missing file.
        for (path, refusal) in refused("Cargo.toml") {
            assert_eq!(block_on(preview_text(path.into(), 100)).unwrap_err(), refusal, "{path}");
            assert_eq!(block_on(preview_bytes(path.into(), 100)).err(), Some(refusal), "{path}");
        }
        for (path, refusal) in refused("src") {
            let found = search_local(path, "lib", false, 10, &mut |_| panic!("searched {path}"), &|| false);
            assert_eq!(found.unwrap_err(), refusal, "{path}");
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "hello").unwrap();
        let file = dir.path().join("notes.txt").to_string_lossy().to_string();
        assert_eq!(block_on(preview_text(file, 100)).unwrap().text, "hello");
        let mut found = Vec::new();
        let root = dir.path().to_string_lossy().to_string();
        search_local(&root, "notes", false, 10, &mut |batch| found.extend(batch), &|| false).unwrap();
        assert_eq!(found.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["notes.txt"]);
    }

    #[test]
    fn open_target_answers_a_local_path_with_itself() {
        // Remote kinds download through their own handler instead (`open_target`).
        let target = open_local_target("/Users/me/a b.txt".into()).unwrap();
        assert_eq!(
            serde_json::to_value(target).unwrap(),
            serde_json::json!({ "kind": "file", "path": "/Users/me/a b.txt" })
        );
        assert_eq!(open_local_target("a.txt".into()).unwrap_err(), "“a.txt” is not a local file.");
    }

    #[test]
    fn open_and_open_with_refuse_other_paths_before_launching_anything() {
        // Neither the file nor the app exists, so without the check these
        // fail on the missing file and never launch anything.
        for (path, refusal) in refused("missing.txt") {
            assert_eq!(block_on(open_file(path.into())).unwrap_err(), refusal, "{path}");
            let with = block_on(open_with(path.into(), "/no/such.desktop".into()));
            assert_eq!(with.unwrap_err(), refusal, "{path}");
        }
    }
}

#[cfg(test)]
mod local_gate_tests {
    //! Keep Versions, sharing a folder and the Trash's details work on this
    //! computer only: every other path, and a relative one (which would be
    //! read from the app's working directory), is refused before anything
    //! is read or written.
    use super::*;

    /// In the tests' working directory (the crate root), so a refusal isn't
    /// a missing folder.
    const RELATIVE: &str = "src";

    /// Each refused path with its refusal.
    fn refused() -> Vec<(&'static str, String)> {
        vec![
            ("share://r1/a.txt", "This action is not available for shared folders.".into()),
            ("sftp://me@host/a.txt", "This action is not available for SSH servers.".into()),
            ("gdrive://acct/root~x/f1~a.txt", "This action is not available for Google Drive items yet.".into()),
            (RELATIVE, format!("“{RELATIVE}” is not a local file.")),
        ]
    }

    fn versions_state(dir: &tempfile::TempDir) -> VersionsState {
        VersionsState::new(dir.path().join("versioned.json"))
    }

    fn text(path: &Path) -> String {
        path.to_string_lossy().to_string()
    }

    #[test]
    fn keep_versions_reads_histories_on_this_computer_only() {
        let dir = tempfile::tempdir().unwrap();
        let state = versions_state(&dir);
        for (path, refusal) in refused() {
            assert_eq!(history_in(&state, path).err(), Some(refusal.clone()), "{path}");
            assert_eq!(history_info_in(&state, path).err(), Some(refusal.clone()), "{path}");
            let trash = |_: &Path| -> Result<(), String> { panic!("trashed the history of {path}") };
            assert_eq!(clear_history_in(&state, path, trash).err(), Some(refusal.clone()), "{path}");
            assert_eq!(stop_keeping_in(&state, path, trash).err(), Some(refusal), "{path}");
        }
        // A folder on this computer still gets its answer.
        let root = dir.path().join("Projects");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a.txt"), "a").unwrap();
        let none = "This folder does not keep versions.".to_string();
        assert_eq!(history_in(&state, &text(&root.join("a.txt"))).err(), Some(none.clone()));
        drop(History::create(&root).unwrap());
        let (_, rel) = history_in(&state, &text(&root.join("a.txt"))).unwrap();
        assert_eq!(rel, PathBuf::from("a.txt"));
        assert_eq!(history_in(&state, &text(&root.join(".versions/meta.json"))).err(), Some(none));
        assert!(history_info_in(&state, &text(&root)).is_ok());
    }

    #[test]
    fn new_versioned_folders_are_made_on_this_computer_only() {
        let dir = tempfile::tempdir().unwrap();
        let state = versions_state(&dir);
        // Parents that don't exist, so nothing is made even without the check.
        for (path, refusal) in [
            ("share://r1/New Folder", "Shared folders are read only."),
            ("sftp://me@host/New Folder", "SSH servers are read only for now."),
            ("gdrive://acct/root~x/~New Folder", "This action is not available for Google Drive items yet."),
            ("no-such-folder/New Folder", "“no-such-folder/New Folder” is not a local file."),
        ] {
            assert_eq!(make_versioned(&state, path.into()).unwrap_err(), refusal, "{path}");
        }
        let folder = dir.path().join("Notes");
        make_versioned(&state, text(&folder)).unwrap();
        assert!(folder.join(".versions").is_dir());
    }

    #[test]
    fn sharing_takes_folders_on_this_computer_only() {
        for (path, _) in refused() {
            assert_eq!(share_source(path).unwrap_err(), "Only folders on this computer can be shared.", "{path}");
        }
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(share_source(&text(dir.path())), Ok(dir.path()));
    }

    #[test]
    fn trash_details_come_from_this_computer_only() {
        // Not `Trash::new`: with `XDG_DATA_HOME` set, that is the user's own
        // Trash, and on macOS `~/.Trash` isn't made for us.
        let dir = tempfile::tempdir().unwrap();
        let (trash, _, items) = trash_ops::temp_trash(dir.path());
        let item = items.join("old.txt");
        std::fs::write(&item, "old").unwrap();
        let mut paths: Vec<String> = refused().into_iter().map(|(path, _)| path.to_string()).collect();
        paths.push(text(&item));
        let infos = trash_infos(&trash, &paths);
        assert_eq!(infos.len(), paths.len());
        assert!(infos[..paths.len() - 1].iter().all(Option::is_none), "{infos:?}");
        assert!(infos[paths.len() - 1].as_ref().is_some_and(|info| info.trashed > 0));
    }
}

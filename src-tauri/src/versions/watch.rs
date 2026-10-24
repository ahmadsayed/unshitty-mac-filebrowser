//! Filesystem watching. One `notify` watcher per owned history, with a small
//! coalescer so an editor's "write temp, rename over the file" becomes one
//! capture. The coalescer is a plain struct with an injected clock so it can
//! be tested without a filesystem.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::event::{EventKind, ModifyKind};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::versions::capture;
use crate::versions::history::History;

const QUIET: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_millis(250);
const FALLBACK_SCAN: Duration = Duration::from_secs(300);

/// Paths whose last event is older than the quiet period are due.
pub struct Coalescer {
    quiet: Duration,
    pending: HashMap<PathBuf, Instant>,
}

impl Coalescer {
    pub fn new(quiet: Duration) -> Self {
        Self {
            quiet,
            pending: HashMap::new(),
        }
    }

    pub fn push(&mut self, path: PathBuf, at: Instant) {
        self.pending.insert(path, at);
    }

    /// Remove and return the paths that have been quiet for `quiet`.
    pub fn due(&mut self, now: Instant) -> Vec<PathBuf> {
        let mut due = Vec::new();
        self.pending.retain(|path, last| {
            if now.saturating_duration_since(*last) >= self.quiet {
                due.push(path.clone());
                false
            } else {
                true
            }
        });
        due
    }
}

pub struct WatcherHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl WatcherHandle {
    pub fn stop(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

/// Start a recursive watcher for `history.root`. The caller keeps the handle;
/// dropping it stops the thread and the watcher.
pub fn start(history: Arc<History>) -> Result<WatcherHandle, String> {
    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .map_err(|e| e.to_string())?;
    watcher
        .watch(&history.root, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;

    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let join = thread::Builder::new()
        .name("versions-watch".into())
        .spawn(move || run(history, rx, watcher, thread_stop))
        .map_err(|e| e.to_string())?;
    Ok(WatcherHandle {
        stop,
        join: Some(join),
    })
}

/// When the platform watcher cannot be created, scan every five minutes
/// instead of watching (the spec's fallback).
pub fn start_fallback(history: Arc<History>) -> WatcherHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let join = thread::Builder::new()
        .name("versions-scan".into())
        .spawn(move || {
            let mut last = Instant::now();
            while !thread_stop.load(Ordering::Relaxed) && !history.is_closed() {
                thread::sleep(Duration::from_secs(1));
                if last.elapsed() >= FALLBACK_SCAN {
                    last = Instant::now();
                    let _ = capture::scan(&history);
                }
            }
        })
        .expect("spawn scan thread");
    WatcherHandle {
        stop,
        join: Some(join),
    }
}

/// What the watcher does with one path from one event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Outside the root, or inside `.versions/` (the app's own writes).
    Ignore,
    /// A file was written: record it once it has been quiet.
    File(PathBuf),
    /// A folder appeared or changed: scan just that folder once it is quiet.
    Folder(PathBuf),
    /// Something was removed or renamed away. It may have been a file or a
    /// folder, so record the path (tombstones a file) and tombstone anything
    /// that was under it.
    Vanished(PathBuf),
}

/// Classify one event path. Pure, so it can be tested without a watcher;
/// `is_dir`/`exists` are what the filesystem says about `path` now.
pub fn classify(root: &Path, path: &Path, kind: &EventKind, is_dir: bool, exists: bool) -> Action {
    let Some(rel) = rel_to(root, path) else {
        return Action::Ignore;
    };
    // The capture itself writes into `.versions/` (tmp files renamed into
    // objects/, the WAL). Those must never trigger more work.
    if crate::versions::is_internal_path(&rel) {
        return Action::Ignore;
    }
    let removed_or_renamed = matches!(
        kind,
        EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    );
    if removed_or_renamed && !exists {
        if rel.as_os_str().is_empty() {
            return Action::Ignore; // the root itself; the state map handles it
        }
        return Action::Vanished(rel);
    }
    if is_dir {
        return Action::Folder(rel);
    }
    Action::File(rel)
}

/// What the watch loop owes the history: files waiting for their quiet
/// period, folders to scan, and whether the whole root needs a scan. Plain
/// state with an injected clock and filesystem probe, so the loop's decisions
/// are tested without a real watcher.
pub struct Pending {
    files: Coalescer,
    folders: Coalescer,
    rescan: bool,
    fallback: bool,
    last_fallback: Instant,
}

/// The work that is due now.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Due {
    /// Files quiet for the whole period: record each one.
    pub files: Vec<PathBuf>,
    /// Folders quiet for the whole period: scan each one (empty when
    /// `full_scan` covers them).
    pub folders: Vec<PathBuf>,
    /// Overflow, or the five-minute fallback after a watcher error.
    pub full_scan: bool,
}

impl Pending {
    pub fn new(now: Instant) -> Self {
        Self {
            files: Coalescer::new(QUIET),
            folders: Coalescer::new(QUIET),
            rescan: false,
            fallback: false,
            last_fallback: now,
        }
    }

    /// Take one message from the watcher. Returns the paths that vanished,
    /// which the caller tombstones at once (with anything under them).
    /// `probe` answers `(is_dir, exists)` for a path.
    pub fn receive(
        &mut self,
        root: &Path,
        message: notify::Result<Event>,
        now: Instant,
        probe: impl Fn(&Path) -> (bool, bool),
    ) -> Vec<PathBuf> {
        let event = match message {
            Ok(event) => event,
            Err(error) => {
                eprintln!("versions: watcher error: {error}");
                self.fallback = true;
                return Vec::new();
            }
        };
        if event.need_rescan() {
            self.rescan = true;
        }
        let mut vanished = Vec::new();
        for path in &event.paths {
            let (is_dir, exists) = probe(path);
            match classify(root, path, &event.kind, is_dir, exists) {
                Action::Ignore => {}
                Action::File(rel) => self.files.push(rel, now),
                Action::Folder(rel) => self.folders.push(rel, now),
                Action::Vanished(rel) => {
                    self.files.push(rel.clone(), now);
                    vanished.push(rel);
                }
            }
        }
        vanished
    }

    /// A file that was still being written: wait another quiet period.
    pub fn retry(&mut self, rel: PathBuf, now: Instant) {
        self.files.push(rel, now);
    }

    pub fn due(&mut self, now: Instant) -> Due {
        let files = self.files.due(now);
        let fallback_due =
            self.fallback && now.saturating_duration_since(self.last_fallback) >= FALLBACK_SCAN;
        if fallback_due {
            self.last_fallback = now;
        }
        if self.rescan || fallback_due {
            self.rescan = false;
            // The full scan covers every pending folder.
            self.folders = Coalescer::new(QUIET);
            return Due {
                files,
                folders: Vec::new(),
                full_scan: true,
            };
        }
        Due {
            files,
            folders: self.folders.due(now),
            full_scan: false,
        }
    }
}

fn run(
    history: Arc<History>,
    rx: mpsc::Receiver<notify::Result<Event>>,
    _watcher: RecommendedWatcher,
    stop: Arc<AtomicBool>,
) {
    let mut pending = Pending::new(Instant::now());
    while !stop.load(Ordering::Relaxed) && !history.is_closed() {
        match rx.recv_timeout(TICK) {
            Ok(message) => {
                let probe = |path: &Path| (path.is_dir(), path.exists());
                for rel in pending.receive(&history.root, message, Instant::now(), probe) {
                    let _ = capture::tombstone_prefix(&history, &rel);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if !do_due_work(&history, &mut pending) {
            return;
        }
    }
}

/// Record the files and scan the folders that are due. Returns false once the
/// history has been closed, which ends the loop.
fn do_due_work(history: &History, pending: &mut Pending) -> bool {
    let due = pending.due(Instant::now());
    for rel in due.files {
        match capture::record(history, &rel) {
            Err(crate::versions::Error::Changed) => pending.retry(rel, Instant::now()),
            Err(crate::versions::Error::Closed) => return false,
            Err(error) => eprintln!("versions: could not capture a file: {error}"),
            Ok(()) => {}
        }
    }
    if due.full_scan {
        let _ = capture::scan(history);
    }
    for rel in due.folders {
        let _ = capture::scan_dir(history, &rel);
    }
    true
}

/// Relative path helper used by tests and the watch loop.
pub fn rel_to(root: &Path, path: &Path) -> Option<PathBuf> {
    path.strip_prefix(root).ok().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_repeated_events_until_quiet() {
        let mut c = Coalescer::new(Duration::from_secs(2));
        let t0 = Instant::now();
        c.push(PathBuf::from("/r/a"), t0);
        c.push(PathBuf::from("/r/b"), t0 + Duration::from_millis(500));
        assert!(c.due(t0 + Duration::from_millis(1500)).is_empty());
        // a is 2 s old; b is 1 s old and must wait.
        let due = c.due(t0 + Duration::from_secs(2));
        assert_eq!(due, vec![PathBuf::from("/r/a")]);
        let due = c.due(t0 + Duration::from_millis(2500));
        assert_eq!(due, vec![PathBuf::from("/r/b")]);
    }

    #[test]
    fn a_new_event_restarts_the_quiet_period() {
        let mut c = Coalescer::new(Duration::from_secs(2));
        let t0 = Instant::now();
        c.push(PathBuf::from("/r/a"), t0);
        c.push(PathBuf::from("/r/a"), t0 + Duration::from_secs(1));
        assert!(c.due(t0 + Duration::from_secs(2)).is_empty());
        assert_eq!(
            c.due(t0 + Duration::from_secs(3)),
            vec![PathBuf::from("/r/a")]
        );
    }

    fn create() -> EventKind {
        EventKind::Create(notify::event::CreateKind::Any)
    }

    fn renamed() -> EventKind {
        EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::Any))
    }

    #[test]
    fn the_apps_own_writes_never_cause_work() {
        let root = Path::new("/r");
        // Every capture renames tmp/obj-… into objects/xx/: the old path is gone.
        let tmp = Path::new("/r/.versions/tmp/obj-1-2");
        assert_eq!(
            classify(root, tmp, &renamed(), false, false),
            Action::Ignore
        );
        let objects = Path::new("/r/.versions/objects/3f");
        assert_eq!(
            classify(root, objects, &create(), true, true),
            Action::Ignore
        );
        let wal = Path::new("/r/.versions/index.sqlite-wal");
        assert_eq!(classify(root, wal, &create(), false, true), Action::Ignore);
        let nested = Path::new("/r/sub/.versions/tmp/x");
        assert_eq!(
            classify(root, nested, &renamed(), false, false),
            Action::Ignore
        );
        assert_eq!(
            classify(root, Path::new("/elsewhere/a"), &create(), false, true),
            Action::Ignore
        );
    }

    #[test]
    fn files_folders_and_vanished_paths_are_told_apart() {
        let root = Path::new("/r");
        assert_eq!(
            classify(root, Path::new("/r/a.txt"), &create(), false, true),
            Action::File(PathBuf::from("a.txt"))
        );
        // A deleted file is not a folder event: no whole-root scan.
        let removed = EventKind::Remove(notify::event::RemoveKind::Any);
        assert_eq!(
            classify(root, Path::new("/r/a.txt"), &removed, false, false),
            Action::Vanished(PathBuf::from("a.txt"))
        );
        assert_eq!(
            classify(root, Path::new("/r/old"), &renamed(), false, false),
            Action::Vanished(PathBuf::from("old"))
        );
        assert_eq!(
            classify(root, Path::new("/r/new"), &renamed(), true, true),
            Action::Folder(PathBuf::from("new"))
        );
        assert_eq!(
            classify(root, Path::new("/r"), &removed, false, false),
            Action::Ignore
        );
    }

    fn event(kind: EventKind, paths: &[&str]) -> notify::Result<Event> {
        let mut event = Event::new(kind);
        for path in paths {
            event = event.add_path(PathBuf::from(path));
        }
        Ok(event)
    }

    /// A probe for a fake filesystem: `dirs` exist as folders, `files` as files.
    fn probe<'a>(dirs: &'a [&str], files: &'a [&str]) -> impl Fn(&Path) -> (bool, bool) + 'a {
        move |path: &Path| {
            let path = path.to_str().unwrap();
            let is_dir = dirs.contains(&path);
            (is_dir, is_dir || files.contains(&path))
        }
    }

    #[test]
    fn a_save_is_recorded_once_after_the_quiet_period() {
        let root = Path::new("/r");
        let t0 = Instant::now();
        let mut pending = Pending::new(t0);
        let files = ["/r/notes.txt"];
        // An editor's save: several events for one file within a second.
        for ms in [0, 300, 900] {
            let message = event(create(), &["/r/notes.txt"]);
            let vanished = pending.receive(
                root,
                message,
                t0 + Duration::from_millis(ms),
                probe(&[], &files),
            );
            assert!(vanished.is_empty());
        }
        assert_eq!(
            pending.due(t0 + Duration::from_millis(2500)),
            Due::default()
        );
        let due = pending.due(t0 + Duration::from_millis(2900));
        assert_eq!(due.files, vec![PathBuf::from("notes.txt")]);
        assert!(!due.full_scan && due.folders.is_empty());
        assert_eq!(pending.due(t0 + Duration::from_secs(10)), Due::default());
    }

    #[test]
    fn a_file_still_being_written_waits_another_period() {
        let t0 = Instant::now();
        let mut pending = Pending::new(t0);
        pending.retry(PathBuf::from("big.bin"), t0);
        assert!(pending.due(t0 + Duration::from_secs(1)).files.is_empty());
        assert_eq!(
            pending.due(t0 + Duration::from_secs(2)).files,
            vec![PathBuf::from("big.bin")]
        );
    }

    #[test]
    fn a_removed_path_is_tombstoned_at_once_and_recorded_later() {
        let root = Path::new("/r");
        let t0 = Instant::now();
        let mut pending = Pending::new(t0);
        let removed = EventKind::Remove(notify::event::RemoveKind::Any);
        let vanished = pending.receive(root, event(removed, &["/r/old"]), t0, probe(&[], &[]));
        assert_eq!(vanished, vec![PathBuf::from("old")]);
        let due = pending.due(t0 + QUIET);
        assert_eq!(due.files, vec![PathBuf::from("old")]);
        assert!(!due.full_scan, "a deletion must not rescan the whole root");
    }

    #[test]
    fn a_new_folder_scans_only_that_folder() {
        let root = Path::new("/r");
        let t0 = Instant::now();
        let mut pending = Pending::new(t0);
        pending.receive(
            root,
            event(create(), &["/r/photos"]),
            t0,
            probe(&["/r/photos"], &[]),
        );
        let due = pending.due(t0 + QUIET);
        assert_eq!(due.folders, vec![PathBuf::from("photos")]);
        assert!(!due.full_scan);
    }

    #[test]
    fn the_apps_own_writes_queue_no_work() {
        let root = Path::new("/r");
        let t0 = Instant::now();
        let mut pending = Pending::new(t0);
        let message = event(
            renamed(),
            &["/r/.versions/tmp/obj-1-2", "/r/.versions/objects/3f/3fab"],
        );
        let vanished = pending.receive(root, message, t0, probe(&["/r/.versions/objects/3f"], &[]));
        assert!(vanished.is_empty());
        assert_eq!(pending.due(t0 + QUIET), Due::default());
    }

    #[test]
    fn overflow_scans_the_whole_root_once() {
        let root = Path::new("/r");
        let t0 = Instant::now();
        let mut pending = Pending::new(t0);
        pending.receive(
            root,
            event(create(), &["/r/photos"]),
            t0,
            probe(&["/r/photos"], &[]),
        );
        let overflow = Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan);
        pending.receive(root, Ok(overflow), t0, probe(&[], &[]));
        let due = pending.due(t0);
        assert!(due.full_scan);
        assert!(
            due.folders.is_empty(),
            "the full scan covers pending folders"
        );
        assert!(!pending.due(t0 + QUIET).full_scan, "only once");
        assert!(pending.due(t0 + QUIET).folders.is_empty());
    }

    #[test]
    fn a_watcher_error_falls_back_to_a_scan_every_five_minutes() {
        let root = Path::new("/r");
        let t0 = Instant::now();
        let mut pending = Pending::new(t0);
        pending.receive(
            root,
            Err(notify::Error::generic("too many watches")),
            t0,
            probe(&[], &[]),
        );
        assert!(!pending.due(t0 + Duration::from_secs(60)).full_scan);
        assert!(pending.due(t0 + FALLBACK_SCAN).full_scan);
        assert!(
            !pending
                .due(t0 + FALLBACK_SCAN + Duration::from_secs(1))
                .full_scan
        );
        assert!(pending.due(t0 + FALLBACK_SCAN * 2).full_scan);
    }

    #[test]
    fn relative_paths_are_only_returned_under_the_root() {
        assert_eq!(
            rel_to(Path::new("/r"), Path::new("/r/a/b")),
            Some(PathBuf::from("a/b"))
        );
        assert_eq!(rel_to(Path::new("/r"), Path::new("/other/a")), None);
    }
}

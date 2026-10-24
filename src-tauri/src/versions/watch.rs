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
            while !thread_stop.load(Ordering::Relaxed) {
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

fn run(
    history: Arc<History>,
    rx: mpsc::Receiver<notify::Result<Event>>,
    _watcher: RecommendedWatcher,
    stop: Arc<AtomicBool>,
) {
    let mut coalescer = Coalescer::new(QUIET);
    let mut directory_event = false;
    let mut fallback = false;
    let mut last_fallback = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(TICK) {
            Ok(Ok(event)) => {
                if event.need_rescan() {
                    directory_event = true;
                }
                for path in event.paths {
                    let is_directory = path.is_dir();
                    let removed_or_renamed = matches!(
                        event.kind,
                        EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
                    );
                    if is_directory || (removed_or_renamed && !path.exists()) {
                        if let Ok(rel) = path.strip_prefix(&history.root) {
                            if !rel.as_os_str().is_empty() {
                                let _ = capture::tombstone_prefix(&history, rel);
                            }
                        }
                        directory_event = true;
                    } else {
                        coalescer.push(path, Instant::now());
                    }
                }
            }
            Ok(Err(error)) => {
                eprintln!("versions: watcher error: {error}");
                fallback = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        for path in coalescer.due(Instant::now()) {
            if let Ok(rel) = path.strip_prefix(&history.root) {
                match capture::record(&history, rel) {
                    Err(crate::versions::Error::Changed) => {
                        // Still being written: wait for another quiet period.
                        coalescer.push(path, Instant::now());
                    }
                    Err(error) => eprintln!("versions: could not capture a file: {error}"),
                    Ok(()) => {}
                }
            }
        }
        if directory_event {
            directory_event = false;
            let _ = capture::scan(&history);
        }
        if fallback && last_fallback.elapsed() >= FALLBACK_SCAN {
            last_fallback = Instant::now();
            let _ = capture::scan(&history);
        }
    }
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

    #[test]
    fn relative_paths_are_only_returned_under_the_root() {
        assert_eq!(
            rel_to(Path::new("/r"), Path::new("/r/a/b")),
            Some(PathBuf::from("a/b"))
        );
        assert_eq!(rel_to(Path::new("/r"), Path::new("/other/a")), None);
    }
}

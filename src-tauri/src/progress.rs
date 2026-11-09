//! Progress and cancel for long operations (copies, moves, Empty Trash,
//! remote opens). Modules take a `&Progress`, add to it as work happens and
//! call `check()` between chunks and items; only `lib.rs` turns it into
//! Tauri channel messages.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::Mutex;

use serde::Serialize;

/// The counters, cancel flag and phases live in the `arbesk` crate, so its
/// long operations report into the same [`Progress`] the app shows.
pub use arbesk::{CANCELLED, Phase, Progress};

/// Copies move data in slices this big, checking for a cancel in between.
pub const SLICE: u64 = 1024 * 1024;

/// What the frontend receives, at most every 100 ms and once at the end.
#[derive(Clone, Serialize, PartialEq, Eq, Debug)]
pub struct ProgressUpdate {
    pub bytes: u64,
    pub items: u64,
    pub total: Option<u64>,
    pub phase: Phase,
    pub done: bool,
}

impl ProgressUpdate {
    /// This reading of `progress`, as the frontend gets it.
    fn of(progress: &Progress, done: bool) -> Self {
        let snapshot = progress.snapshot();
        Self {
            bytes: snapshot.bytes,
            items: snapshot.items,
            total: snapshot.total,
            phase: snapshot.phase,
            done,
        }
    }
}

/// An `io::Error` carrying `CANCELLED`, for code that speaks `io::Result`.
pub fn cancelled_io() -> io::Error {
    io::Error::other(CANCELLED)
}

/// Whether `error` is the cancel from `cancelled_io`.
pub fn is_cancel(error: &io::Error) -> bool {
    error.to_string() == CANCELLED
}

/// Copy `reader` to `writer` in `SLICE`s, counting bytes and stopping with
/// a cancel error between slices.
pub fn copy_counted(
    reader: &mut impl Read,
    writer: &mut impl Write,
    progress: &Progress,
) -> io::Result<u64> {
    let mut total = 0;
    loop {
        if progress.is_cancelled() {
            return Err(cancelled_io());
        }
        let copied = io::copy(&mut reader.by_ref().take(SLICE), writer)?;
        if copied == 0 {
            return Ok(total);
        }
        total += copied;
        progress.add_bytes(copied);
    }
}

/// Send `progress` every `every` until `stop` disconnects (its sender is
/// dropped), then once more with `done: true`. Runs on its own thread next
/// to the work it reports on, and wakes at once when the work ends.
pub fn ticker(
    progress: &Progress,
    send: impl Fn(ProgressUpdate),
    stop: &std::sync::mpsc::Receiver<()>,
    every: std::time::Duration,
) {
    while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = stop.recv_timeout(every) {
        send(ProgressUpdate::of(progress, false));
    }
    send(ProgressUpdate::of(progress, true));
}

/// The running operations by id, so `cancel_operation` can reach them.
#[derive(Default)]
pub struct Registry(Mutex<HashMap<String, Progress>>);

impl Registry {
    /// The operation's progress, created on first use; a batch shares one.
    pub fn register(&self, op: &str) -> Progress {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(op.to_string()).or_default().clone()
    }

    /// Cancel `op`. Between two items of a batch nothing is registered, so
    /// an unknown op is remembered as cancelled and its next command stops.
    pub fn cancel(&self, op: &str) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(op.to_string()).or_default().cancel();
    }

    pub fn remove(&self, op: &str) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(op);
    }

    /// No operation runs in any window (the registry is app-wide).
    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    #[test]
    fn the_registry_knows_whether_anything_runs_in_any_window() {
        let registry = Registry::default();
        assert!(registry.is_empty());
        registry.register("op-a");
        assert!(!registry.is_empty());
        registry.remove("op-a");
        assert!(registry.is_empty());
    }

    #[test]
    fn counts_bytes_items_and_total() {
        let progress = Progress::default();
        progress.add_bytes(3);
        progress.add_bytes(4);
        progress.add_item();
        progress.set_total(10);
        let snapshot = progress.snapshot();
        assert_eq!(
            (snapshot.bytes, snapshot.items, snapshot.total, snapshot.phase),
            (7, 1, Some(10), Phase::Preparing)
        );
        assert_eq!(
            ProgressUpdate::of(&progress, false),
            ProgressUpdate {
                bytes: 7,
                items: 1,
                total: Some(10),
                phase: Phase::Preparing,
                done: false,
            }
        );
    }

    #[test]
    fn each_phase_is_reported_by_its_name() {
        let progress = Progress::default();
        for (phase, name) in [
            (Phase::Working, "\"working\""),
            (Phase::Saving, "\"saving\""),
            (Phase::Preparing, "\"preparing\""),
        ] {
            progress.set_phase(phase);
            assert_eq!(progress.snapshot().phase, phase);
            assert_eq!(serde_json::to_string(&phase).unwrap(), name);
        }
    }

    #[test]
    fn check_fails_with_cancelled_after_cancel() {
        let progress = Progress::default();
        let clone = progress.clone();
        assert!(progress.check().is_ok());
        progress.cancel();
        assert_eq!(progress.check(), Err("Cancelled.".to_string()));
        assert!(clone.is_cancelled());
    }

    #[test]
    fn copy_counted_copies_and_counts() {
        let data: Vec<u8> = (0..3 * 1024 * 1024 + 5).map(|i| (i % 251) as u8).collect();
        let progress = Progress::default();
        let mut out = Vec::new();
        let copied = copy_counted(&mut Cursor::new(&data), &mut out, &progress).unwrap();
        assert_eq!(copied, data.len() as u64);
        assert_eq!(out, data);
        assert_eq!(progress.snapshot().bytes, data.len() as u64);
    }

    /// A reader that cancels its progress once the first slice has been read.
    struct CancelAfterFirst {
        inner: Cursor<Vec<u8>>,
        progress: Progress,
    }

    impl Read for CancelAfterFirst {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.progress.snapshot().bytes > 0 {
                self.progress.cancel();
            }
            self.inner.read(buf)
        }
    }

    #[test]
    fn copy_counted_stops_when_cancelled() {
        let size = 4 * 1024 * 1024;
        let progress = Progress::default();
        let mut reader = CancelAfterFirst {
            inner: Cursor::new(vec![7u8; size]),
            progress: progress.clone(),
        };
        let mut out = Vec::new();
        let error = copy_counted(&mut reader, &mut out, &progress).unwrap_err();
        assert_eq!(error.to_string(), "Cancelled.");
        assert!(out.len() < size);
    }

    #[test]
    fn registry_shares_one_progress_per_op_and_cancels_it() {
        let registry = Registry::default();
        let first = registry.register("a");
        let second = registry.register("a");
        registry.cancel("a");
        assert!(first.is_cancelled());
        assert!(second.is_cancelled());
        registry.cancel("missing");
        registry.remove("a");
        assert!(!registry.register("a").is_cancelled());
    }

    #[test]
    fn ticker_sends_a_final_done_update() {
        let sent = std::sync::Mutex::new(Vec::new());
        let (stopper, stop) = std::sync::mpsc::channel::<()>();
        drop(stopper);
        let begun = std::time::Instant::now();
        ticker(
            &Progress::default(),
            |update| sent.lock().unwrap().push(update),
            &stop,
            std::time::Duration::from_millis(50),
        );
        assert!(begun.elapsed() < std::time::Duration::from_millis(50));
        let sent = sent.into_inner().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].done);
    }

    #[test]
    fn ticker_sends_while_running() {
        let sent = std::sync::Mutex::new(Vec::new());
        let (sender, stop) = std::sync::mpsc::channel::<()>();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            drop(sender);
        });
        let progress = Progress::default();
        progress.add_bytes(9);
        ticker(
            &progress,
            |update| sent.lock().unwrap().push(update),
            &stop,
            std::time::Duration::from_millis(50),
        );
        stopper.join().unwrap();
        let sent = sent.into_inner().unwrap();
        assert!(sent.len() >= 3, "only {} updates", sent.len());
        assert!(sent[..sent.len() - 1].iter().all(|u| !u.done && u.bytes == 9));
        assert!(sent.last().unwrap().done);
    }

    /// Each item of a batch waits for its ticker, so stopping must not wait
    /// out the period.
    #[test]
    fn ticker_stops_without_waiting_out_its_period() {
        let (sender, stop) = std::sync::mpsc::channel::<()>();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            drop(sender);
        });
        let begun = std::time::Instant::now();
        ticker(&Progress::default(), |_| {}, &stop, std::time::Duration::from_secs(10));
        stopper.join().unwrap();
        assert!(begun.elapsed() < std::time::Duration::from_secs(1));
    }

    /// Between two items of a batch no command is registered; a cancel sent
    /// then must still stop the next item.
    #[test]
    fn a_cancel_before_register_reaches_the_next_command() {
        let registry = Registry::default();
        registry.cancel("b");
        assert!(registry.register("b").is_cancelled());
    }
}

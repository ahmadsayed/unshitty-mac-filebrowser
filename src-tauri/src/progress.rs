//! Progress and cancel for long operations (copies, moves, Empty Trash,
//! remote opens). Modules take a `&Progress`, add to it as work happens and
//! call `check()` between chunks and items; only `lib.rs` turns it into
//! Tauri channel messages.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;

/// The error a cancelled operation ends with; the frontend treats it as a
/// cancel, not a failure.
pub const CANCELLED: &str = "Cancelled.";

/// Copies move data in slices this big, checking for a cancel in between.
pub const SLICE: u64 = 1024 * 1024;

/// No total known yet.
const NO_TOTAL: u64 = u64::MAX;

#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Preparing,
    Working,
}

/// What the frontend receives, at most every 100 ms and once at the end.
#[derive(Clone, Serialize, PartialEq, Eq, Debug)]
pub struct ProgressUpdate {
    pub bytes: u64,
    pub items: u64,
    pub total: Option<u64>,
    pub phase: Phase,
    pub done: bool,
}

#[derive(Debug)]
struct Counters {
    bytes: AtomicU64,
    items: AtomicU64,
    total: AtomicU64,
    working: AtomicU8,
    cancelled: AtomicBool,
}

impl Default for Counters {
    fn default() -> Self {
        Counters {
            bytes: AtomicU64::new(0),
            items: AtomicU64::new(0),
            total: AtomicU64::new(NO_TOTAL),
            working: AtomicU8::new(0),
            cancelled: AtomicBool::new(false),
        }
    }
}

/// Shared counters and a cancel flag; clones see the same values.
#[derive(Clone, Default, Debug)]
pub struct Progress(Arc<Counters>);

impl Progress {
    pub fn add_bytes(&self, n: u64) {
        self.0.bytes.fetch_add(n, Ordering::Relaxed);
    }

    pub fn set_bytes(&self, n: u64) {
        self.0.bytes.store(n, Ordering::Relaxed);
    }

    pub fn add_item(&self) {
        self.0.items.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_total(&self, bytes: u64) {
        self.0.total.store(bytes, Ordering::Relaxed);
    }

    pub fn set_phase(&self, phase: Phase) {
        self.0
            .working
            .store(u8::from(phase == Phase::Working), Ordering::Relaxed);
    }

    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// `Err("Cancelled.")` once cancelled; call between chunks and items.
    pub fn check(&self) -> Result<(), String> {
        if self.is_cancelled() {
            Err(CANCELLED.into())
        } else {
            Ok(())
        }
    }

    pub fn snapshot(&self) -> ProgressUpdate {
        let total = self.0.total.load(Ordering::Relaxed);
        ProgressUpdate {
            bytes: self.0.bytes.load(Ordering::Relaxed),
            items: self.0.items.load(Ordering::Relaxed),
            total: (total != NO_TOTAL).then_some(total),
            phase: if self.0.working.load(Ordering::Relaxed) == 1 {
                Phase::Working
            } else {
                Phase::Preparing
            },
            done: false,
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

/// The running operations by id, so `cancel_operation` can reach them.
#[derive(Default)]
pub struct Registry(Mutex<HashMap<String, Progress>>);

impl Registry {
    /// The operation's progress, created on first use; a batch shares one.
    pub fn register(&self, op: &str) -> Progress {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(op.to_string()).or_default().clone()
    }

    pub fn cancel(&self, op: &str) {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(progress) = map.get(op) {
            progress.cancel();
        }
    }

    pub fn remove(&self, op: &str) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(op);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    #[test]
    fn counts_bytes_items_and_total() {
        let progress = Progress::default();
        progress.add_bytes(3);
        progress.add_bytes(4);
        progress.add_item();
        progress.set_total(10);
        assert_eq!(
            progress.snapshot(),
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
}

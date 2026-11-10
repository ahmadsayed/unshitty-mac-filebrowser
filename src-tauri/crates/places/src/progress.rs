//! Progress and cancel for long work (copies, moves, downloads, Empty Trash,
//! Arbesk builds and saves): the app shares one [`Progress`] among the
//! operations it reports on, and every provider counts bytes and items into
//! it, sets the phase, and stops when cancelled.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use serde::Serialize;

/// The error a cancelled operation ends with; the app shows it as a cancel,
/// not a failure.
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
    /// The work is done and a server is saving it (Arbesk's relay).
    Saving,
}

impl Phase {
    /// The phase stored as `phase as u8`.
    fn from_u8(stored: u8) -> Self {
        [Phase::Preparing, Phase::Working, Phase::Saving]
            .into_iter()
            .find(|phase| *phase as u8 == stored)
            .unwrap_or(Phase::Preparing)
    }
}

/// One reading of a [`Progress`].
#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
pub struct Snapshot {
    pub bytes: u64,
    pub items: u64,
    pub total: Option<u64>,
    pub phase: Phase,
}

#[derive(Debug)]
struct Counters {
    bytes: AtomicU64,
    items: AtomicU64,
    total: AtomicU64,
    phase: AtomicU8,
    cancelled: AtomicBool,
}

impl Default for Counters {
    fn default() -> Self {
        Counters {
            bytes: AtomicU64::new(0),
            items: AtomicU64::new(0),
            total: AtomicU64::new(NO_TOTAL),
            phase: AtomicU8::new(Phase::Preparing as u8),
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
        self.0.phase.store(phase as u8, Ordering::Relaxed);
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

    pub fn snapshot(&self) -> Snapshot {
        let total = self.0.total.load(Ordering::Relaxed);
        Snapshot {
            bytes: self.0.bytes.load(Ordering::Relaxed),
            items: self.0.items.load(Ordering::Relaxed),
            total: (total != NO_TOTAL).then_some(total),
            phase: Phase::from_u8(self.0.phase.load(Ordering::Relaxed)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

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
        progress.set_bytes(2);
        assert_eq!(progress.snapshot().bytes, 2);
        assert_eq!(Progress::default().snapshot().total, None);
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
    fn a_snapshot_serialises_as_before() {
        let progress = Progress::default();
        progress.add_bytes(5);
        assert_eq!(
            serde_json::to_string(&progress.snapshot()).unwrap(),
            r#"{"bytes":5,"items":0,"total":null,"phase":"preparing"}"#
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
        assert!(is_cancel(&cancelled_io()));
        assert!(!is_cancel(&io::Error::other("Disk full")));
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
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
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
}

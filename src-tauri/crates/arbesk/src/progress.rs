//! Progress and cancel for the crate's long work: the host app shares one
//! [`Progress`] among the operations it reports on, and this crate counts
//! bytes and items into it, sets the phase and stops when cancelled.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::Arc;

use serde::Serialize;

/// The error a cancelled operation ends with; the app shows it as a cancel,
/// not a failure.
pub const CANCELLED: &str = "Cancelled.";

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

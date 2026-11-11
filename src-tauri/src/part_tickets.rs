//! Cancel tickets for the preview's Arbesk parts (performance plan, phase
//! 3), as thumbnails have theirs (`thumbnails::Requests`): a running
//! `arbesk_part` is registered under its window and ticket with the
//! `Progress` its fetch listens to, so `cancel_arbesk_parts` stops it
//! whether it waits for a read permit of Arbesk's limiter or is reading.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::progress::Progress;

/// The parts being fetched, by window and ticket.
#[derive(Clone, Default)]
pub struct PartTickets {
    running: Arc<Mutex<HashMap<(String, u64), Progress>>>,
}

/// One running part; it is forgotten when this is dropped.
pub struct PartTicket {
    tickets: PartTickets,
    key: (String, u64),
    progress: Progress,
}

impl PartTickets {
    fn lock(&self) -> MutexGuard<'_, HashMap<(String, u64), Progress>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register `window`'s part `ticket`, with a fresh `Progress`.
    pub fn start(&self, window: &str, ticket: u64) -> PartTicket {
        let key = (window.to_string(), ticket);
        let progress = Progress::default();
        self.lock().insert(key.clone(), progress.clone());
        PartTicket { tickets: self.clone(), key, progress }
    }

    /// Cancel `window`'s parts with these tickets; finished ones are ignored.
    pub fn cancel(&self, window: &str, tickets: &[u64]) {
        let running = self.lock();
        for ticket in tickets {
            if let Some(progress) = running.get(&(window.to_string(), *ticket)) {
                progress.cancel();
            }
        }
    }
}

impl PartTicket {
    /// What the part's fetch listens to.
    pub fn progress(&self) -> Progress {
        self.progress.clone()
    }
}

impl Drop for PartTicket {
    fn drop(&mut self) {
        self.tickets.lock().remove(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_reaches_its_windows_running_part_only() {
        let tickets = PartTickets::default();
        let mine = tickets.start("main", 1);
        let other = tickets.start("main", 2);
        let elsewhere = tickets.start("second", 1);
        tickets.cancel("main", &[1, 99]);
        assert!(mine.progress().is_cancelled());
        assert!(!other.progress().is_cancelled());
        assert!(!elsewhere.progress().is_cancelled());
    }

    #[test]
    fn a_finished_part_is_forgotten() {
        let tickets = PartTickets::default();
        let progress = tickets.start("main", 1).progress();
        assert!(tickets.lock().is_empty());
        // A cancel after it ended reaches nothing.
        tickets.cancel("main", &[1]);
        assert!(!progress.is_cancelled());
    }
}

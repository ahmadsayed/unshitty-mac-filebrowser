//! Cancel tickets for the preview's Arbesk parts (performance plan, phase
//! 3), as thumbnails have theirs (`thumbnails::Requests`): a running
//! `arbesk_part` is registered under its window and ticket with the
//! `Progress` its fetch listens to, so `cancel_arbesk_parts` stops it
//! whether it waits for a read permit of Arbesk's limiter or is reading.
//! A copy out's builds ahead (`arbesk_prefetch`, phase 5) take tickets
//! here too, from the same frontend counter, so the same cancel stops them.
//!
//! `arbesk_part` is async: its ticket is registered when the runtime first
//! runs it, while `cancel_arbesk_parts` runs at once, so a cancel can come
//! before the part it names. Such a cancel is kept (the last
//! [`EARLY_CANCELS`]) and the part starts cancelled. The frontend numbers
//! its tickets from the clock at load, so a reload never reuses one.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::progress::Progress;

/// How many cancels of parts not started yet are kept.
const EARLY_CANCELS: usize = 64;

/// The parts being fetched, by window and ticket.
#[derive(Clone, Default)]
pub struct PartTickets {
    tickets: Arc<Mutex<Tickets>>,
}

#[derive(Default)]
struct Tickets {
    running: HashMap<(String, u64), Progress>,
    /// Cancels that came before their part started, oldest first.
    early: VecDeque<(String, u64)>,
}

/// One running part; it is forgotten when this is dropped.
pub struct PartTicket {
    tickets: PartTickets,
    key: (String, u64),
    progress: Progress,
}

impl PartTickets {
    fn lock(&self) -> MutexGuard<'_, Tickets> {
        self.tickets.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register `window`'s part `ticket`, with a fresh `Progress`, already
    /// cancelled when its cancel came first.
    pub fn start(&self, window: &str, ticket: u64) -> PartTicket {
        let key = (window.to_string(), ticket);
        let progress = Progress::default();
        let mut tickets = self.lock();
        if let Some(at) = tickets.early.iter().position(|early| *early == key) {
            tickets.early.remove(at);
            progress.cancel();
        }
        tickets.running.insert(key.clone(), progress.clone());
        drop(tickets);
        PartTicket { tickets: self.clone(), key, progress }
    }

    /// Cancel `window`'s parts with these tickets; one not running is kept
    /// for its start (a finished one's never comes, and ages out).
    pub fn cancel(&self, window: &str, tickets: &[u64]) {
        let mut all = self.lock();
        for ticket in tickets {
            let key = (window.to_string(), *ticket);
            match all.running.get(&key) {
                Some(progress) => progress.cancel(),
                None if !all.early.contains(&key) => {
                    if all.early.len() >= EARLY_CANCELS {
                        all.early.pop_front();
                    }
                    all.early.push_back(key);
                }
                None => {}
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
        self.tickets.lock().running.remove(&self.key);
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
        assert!(tickets.lock().running.is_empty());
        // A cancel after it ended reaches nothing.
        tickets.cancel("main", &[1]);
        assert!(!progress.is_cancelled());
    }

    /// The async `arbesk_part` registers its ticket only once it runs; a
    /// cancel sent right after the call can overtake it.
    #[test]
    fn a_cancel_before_its_part_starts_starts_it_cancelled() {
        let tickets = PartTickets::default();
        tickets.cancel("main", &[7]);
        assert!(tickets.start("main", 7).progress().is_cancelled());
        // Spent: the same ticket again, or another window's, starts afresh.
        assert!(!tickets.start("main", 7).progress().is_cancelled());
        tickets.cancel("second", &[8]);
        assert!(!tickets.start("main", 8).progress().is_cancelled());
    }

    #[test]
    fn early_cancels_are_bounded() {
        let tickets = PartTickets::default();
        let many: Vec<u64> = (0..EARLY_CANCELS as u64 + 10).collect();
        tickets.cancel("main", &many);
        tickets.cancel("main", &[20]);
        assert_eq!(tickets.lock().early.len(), EARLY_CANCELS);
        // The oldest aged out; the newest are kept.
        assert!(!tickets.start("main", 0).progress().is_cancelled());
        assert!(tickets.start("main", EARLY_CANCELS as u64 + 9).progress().is_cancelled());
    }
}

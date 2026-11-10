//! Progress and cancel for long operations (copies, moves, Empty Trash,
//! remote opens). Modules take a `&Progress`, add to it as work happens and
//! call `check()` between chunks and items; only `lib.rs` turns it into
//! Tauri channel messages.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;

/// The counters, cancel flag, phases and the counted copy live in the
/// `places` crate, so every provider reports into the same [`Progress`] the
/// app shows.
pub use places::{CANCELLED, Phase, Progress, cancelled_io, copy_counted, is_cancel};
/// Kept at its old path like the rest, for code written against it; no
/// module here reads it today.
#[allow(unused_imports)]
pub use places::SLICE;

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

/// One command's part of operation `op` while it runs: its progress,
/// registered in `registry` so `cancel_operation` reaches it, and a ticker
/// thread sending it through `send` every `every`. Dropping it, when the
/// work ends or unwinds from a panic, stops the ticker (which sends a last
/// update with `done`) and forgets `op`, so an operation that failed never
/// keeps `restart_app` refusing.
pub struct Tracked<'r> {
    registry: &'r Registry,
    op: String,
    pub progress: Progress,
    stop: Option<std::sync::mpsc::Sender<()>>,
    ticker: Option<std::thread::JoinHandle<()>>,
}

impl<'r> Tracked<'r> {
    pub fn start(
        registry: &'r Registry,
        op: &str,
        send: impl Fn(ProgressUpdate) + Send + 'static,
        every: std::time::Duration,
    ) -> Self {
        let progress = registry.register(op);
        progress.set_phase(Phase::Working);
        let (stop, stopped) = std::sync::mpsc::channel();
        let ticker = {
            let progress = progress.clone();
            std::thread::spawn(move || ticker(&progress, send, &stopped, every))
        };
        Tracked { registry, op: op.to_string(), progress, stop: Some(stop), ticker: Some(ticker) }
    }

    /// The work is over: stop the ticker and forget `op` (what dropping does).
    pub fn finish(self) {}
}

impl Drop for Tracked<'_> {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(ticker) = self.ticker.take() {
            let _ = ticker.join();
        }
        self.registry.remove(&self.op);
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
    fn a_finished_operation_is_forgotten_after_its_last_update() {
        let registry = Registry::default();
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let tracked = {
            let sent = sent.clone();
            Tracked::start(&registry, "op", move |u| sent.lock().unwrap().push(u), std::time::Duration::from_secs(10))
        };
        assert!(!registry.is_empty());
        assert_eq!(tracked.progress.snapshot().phase, Phase::Working);
        tracked.finish();
        assert!(registry.is_empty());
        assert!(sent.lock().unwrap().last().unwrap().done);
    }

    /// A panic in the work must not leave `op` registered: `restart_app`
    /// would refuse for the rest of the session.
    #[test]
    fn an_operation_whose_work_panics_is_forgotten_too() {
        let registry = Registry::default();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _tracked = Tracked::start(&registry, "op", |_| {}, std::time::Duration::from_secs(10));
            panic!("the fetch failed");
        }));
        assert!(unwound.is_err());
        assert!(registry.is_empty());
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

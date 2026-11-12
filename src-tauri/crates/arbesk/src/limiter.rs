//! One limit for every Arbesk read (performance plan, phase 2): a
//! [`Limiter`] per [`crate::Client`], shared by listings, parts,
//! thumbnails, an asset's other files and version walks, so the features
//! never add up their own caps. At most [`READS`] gateway reads are in
//! flight, and [`RPCS`] chain RPC calls beside them. A read waiting for a
//! permit listens to its operation's stop flag, so Cancel and a closed
//! preview leave the queue at once. Two callers asking for the same CID at
//! once share one read ([`Limiter::shared`]).
//!
//! The limits reach the reads through the [`Http`] they are handed:
//! [`Limited`] wraps the client's and answers [`Http::limits`], which the
//! gateway (`ipfs::Gateway`) and the chain's `eth_call` ask before each
//! request. A write's reads go through the bare client, unlimited, as
//! before: a save under its collection's hold never queues behind a
//! listing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::http::{Http, HttpError, TICK};
use crate::progress::CANCELLED;

/// How many gateway reads are in flight at once, across the whole app.
pub const READS: usize = 8;
/// How many chain RPC calls are in flight at once, across the whole app.
pub const RPCS: usize = 4;
/// How many bytes of parts every build together may hold or have on their
/// way before another part of known size starts (`compose::Window`); each
/// build still refuses past its own cap.
pub const HELD: u64 = 512 * 1024 * 1024;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A stop flag that may be asked from any thread.
pub type Stop<'a> = &'a (dyn Fn() -> bool + Sync);

/// Never stops: a listing, a scene or a version walk has no Cancel.
pub fn never() -> bool {
    false
}

/// Permits for one kind of request.
struct Pool {
    cap: usize,
    used: Mutex<usize>,
    freed: Condvar,
}

impl Pool {
    fn new(cap: usize) -> Self {
        Self {
            cap,
            used: Mutex::new(0),
            freed: Condvar::new(),
        }
    }

    /// A permit, once one is free; [`CANCELLED`] as soon as `stop` says so
    /// (heard every [`TICK`] while waiting), before anything is asked.
    fn take(self: &Arc<Self>, stop: Stop) -> Result<Permit, String> {
        let mut used = lock(&self.used);
        loop {
            if stop() {
                return Err(CANCELLED.into());
            }
            if *used < self.cap {
                *used += 1;
                return Ok(Permit { pool: self.clone() });
            }
            used = match self.freed.wait_timeout(used, TICK) {
                Ok((used, _)) => used,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }
}

/// One request's place in a [`Pool`], given back when dropped. It owns
/// its place (not borrowed from the [`Limiter`]), so a part's read can hand
/// it to the thread that holds the connection (`http::pump`): a read
/// cancelled while the gateway is silent keeps its place until that
/// connection is really closed (performance plan, phase 6).
pub struct Permit {
    pool: Arc<Pool>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        *lock(&self.pool.used) -= 1;
        self.pool.freed.notify_one();
    }
}

/// One read several callers wait for: its answer once it has one.
#[derive(Default)]
struct Flight {
    answer: Mutex<Option<Result<Arc<Vec<u8>>, String>>>,
    landed: Condvar,
}

/// The app's limits on Arbesk reads: one per [`crate::Client`].
pub struct Limiter {
    reads: Arc<Pool>,
    rpcs: Arc<Pool>,
    /// The bytes of parts builds hold or have on their way, and the most
    /// they may before another part of known size starts.
    held: Mutex<u64>,
    held_cap: u64,
    /// Reads under way, by what they read (a gateway URL, or a token's
    /// `tokenURI` for [`crate::memo::TokenMemo`]).
    flights: Mutex<HashMap<String, Arc<Flight>>>,
    /// The `tokenURI` answers reads share for a few seconds (performance
    /// plan, phase 4). Only reads through [`Limited`] reach it: a write's
    /// bare client never does.
    pub tokens: crate::memo::TokenMemo,
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new(READS, RPCS)
    }
}

impl Limiter {
    pub fn new(reads: usize, rpcs: usize) -> Self {
        Self {
            reads: Arc::new(Pool::new(reads.max(1))),
            rpcs: Arc::new(Pool::new(rpcs.max(1))),
            held: Mutex::new(0),
            held_cap: HELD,
            flights: Mutex::new(HashMap::new()),
            tokens: crate::memo::TokenMemo::default(),
        }
    }

    /// The same limits with another shared byte budget (tests use small ones).
    #[cfg(test)]
    pub fn with_held(self, held_cap: u64) -> Self {
        Self { held_cap, ..self }
    }

    /// Hold `bytes` of the builds' shared budget, if they fit beside what
    /// is held, or if the build asking has nothing on its way (`alone`): a
    /// build may always fetch one part, so builds never wait on each other
    /// for ever. False leaves nothing held; the build asks again later.
    pub fn hold(&self, bytes: u64, alone: bool) -> bool {
        let mut held = lock(&self.held);
        let fits = held.saturating_add(bytes) <= self.held_cap;
        if fits || alone {
            *held = held.saturating_add(bytes);
        }
        fits || alone
    }

    /// Give back `bytes` held with [`Limiter::hold`].
    pub fn let_go(&self, bytes: u64) {
        let mut held = lock(&self.held);
        *held = held.saturating_sub(bytes);
    }

    /// How many bytes the builds hold now (tests).
    #[cfg(test)]
    pub fn holding(&self) -> u64 {
        *lock(&self.held)
    }

    /// A gateway read's permit (see [`Pool::take`]).
    pub fn read(&self, stop: Stop) -> Result<Permit, String> {
        self.reads.take(stop)
    }

    /// A chain RPC call's permit (see [`Pool::take`]).
    pub fn rpc(&self, stop: Stop) -> Result<Permit, String> {
        self.rpcs.take(stop)
    }

    /// How many callers wait for the read of `key` under way (tests).
    #[cfg(test)]
    pub fn followers(&self, key: &str) -> usize {
        // The map's own `Arc`, the leader's, and one per follower.
        lock(&self.flights).get(key).map_or(0, |flight| Arc::strong_count(flight).saturating_sub(2))
    }

    /// How many gateway reads hold a permit now (tests).
    #[cfg(test)]
    pub fn reading(&self) -> usize {
        *lock(&self.reads.used)
    }

    /// `read`'s answer for `key`, shared: a caller that asks while another
    /// read of `key` is under way waits for that one's answer instead of
    /// reading again. A waiting caller hears its own `stop`; when the read
    /// it waits for was itself cancelled (its caller's stop), it reads again
    /// for itself. The answer is only shared while the read is under way:
    /// a later caller reads again.
    pub fn shared(
        &self,
        key: &str,
        stop: Stop,
        read: impl Fn() -> Result<Vec<u8>, String>,
    ) -> Result<Vec<u8>, String> {
        loop {
            let (flight, leads) = {
                let mut flights = lock(&self.flights);
                match flights.get(key) {
                    Some(flight) => (flight.clone(), false),
                    None => {
                        let flight = Arc::new(Flight::default());
                        flights.insert(key.to_string(), flight.clone());
                        (flight, true)
                    }
                }
            };
            if leads {
                return self.lead(key, &flight, &read);
            }
            match wait(&flight, stop)? {
                Ok(bytes) => return Ok(bytes.as_ref().clone()),
                Err(error) if error == CANCELLED => continue,
                Err(error) => return Err(error),
            }
        }
    }

    /// Run `read` for everyone waiting on `flight`, then let them have it.
    /// A read nobody else waits for is handed back as it is, never copied:
    /// the answer is only published (and each follower copies it) when
    /// someone joined the flight while it was under way.
    fn lead(
        &self,
        key: &str,
        flight: &Arc<Flight>,
        read: &dyn Fn() -> Result<Vec<u8>, String>,
    ) -> Result<Vec<u8>, String> {
        /// Lands the flight if `read` panics, so nobody waits for ever.
        struct Landing<'a> {
            limiter: &'a Limiter,
            key: &'a str,
            flight: &'a Flight,
            armed: bool,
        }
        impl Drop for Landing<'_> {
            fn drop(&mut self) {
                if self.armed {
                    lock(&self.limiter.flights).remove(self.key);
                    *lock(&self.flight.answer) = Some(Err("The read stopped unexpectedly.".into()));
                    self.flight.landed.notify_all();
                }
            }
        }
        let mut landing = Landing {
            limiter: self,
            key,
            flight,
            armed: true,
        };
        let answer = read();
        landing.armed = false;
        // Once the key is gone nobody new can join, so the count is final:
        // the leader's own `Arc` and one per follower.
        let followed = {
            let mut flights = lock(&self.flights);
            flights.remove(key);
            Arc::strong_count(flight) > 1
        };
        if !followed {
            return answer;
        }
        let answer = answer.map(Arc::new);
        *lock(&flight.answer) = Some(answer.clone());
        flight.landed.notify_all();
        answer.map(Arc::unwrap_or_clone)
    }
}

/// The answer `flight` lands with, or [`CANCELLED`] once `stop` says so.
fn wait(flight: &Flight, stop: Stop) -> Result<Result<Arc<Vec<u8>>, String>, String> {
    let mut answer = lock(&flight.answer);
    loop {
        if let Some(answer) = answer.as_ref() {
            return Ok(answer.clone());
        }
        if stop() {
            return Err(CANCELLED.into());
        }
        answer = match flight.landed.wait_timeout(answer, TICK) {
            Ok((answer, _)) => answer,
            Err(poisoned) => poisoned.into_inner().0,
        };
    }
}

/// What a read is limited by: the app's [`Limiter`] and its operation's stop.
#[derive(Clone, Copy)]
pub struct Limits<'a> {
    pub limiter: &'a Limiter,
    pub stop: Stop<'a>,
}

/// An [`Http`] whose reads go through `limits` (see [`Http::limits`]); every
/// request is the inner client's.
pub struct Limited<'a> {
    inner: &'a dyn Http,
    limits: Limits<'a>,
}

impl<'a> Limited<'a> {
    pub fn new(inner: &'a dyn Http, limiter: &'a Limiter, stop: Stop<'a>) -> Self {
        Self {
            inner,
            limits: Limits { limiter, stop },
        }
    }
}

impl Http for Limited<'_> {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        self.inner.get(url)
    }

    fn get_streamed(
        &self,
        url: &str,
        expected: u64,
        chunk: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), HttpError> {
        self.inner.get_streamed(url, expected, chunk)
    }

    fn get_chunked(
        &self,
        url: &str,
        cap: Option<u64>,
        held: crate::http::Held,
        chunk: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), HttpError> {
        self.inner.get_chunked(url, cap, held, chunk)
    }

    fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError> {
        self.inner.post_json(url, body, session)
    }

    fn post_json_slow(
        &self,
        url: &str,
        body: &serde_json::Value,
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError> {
        self.inner.post_json_slow(url, body, session)
    }

    fn post_multipart(
        &self,
        url: &str,
        file_name: &str,
        bytes: &[u8],
        fields: &[(&str, &str)],
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError> {
        self.inner.post_multipart(url, file_name, bytes, fields, session)
    }

    fn delete(&self, url: &str, session: Option<&str>) -> Result<(), HttpError> {
        self.inner.delete(url, session)
    }

    fn limits(&self) -> Option<Limits<'_>> {
        Some(self.limits)
    }
}

/// `work`, handed a stop any thread may ask, for a `stop` only this thread
/// may: `work` runs on a thread of its own while this one asks `stop` every
/// [`TICK`] and passes a yes on. A thumbnail's stop is such a flag.
pub fn bridged<R: Send>(
    stop: &dyn Fn() -> bool,
    work: impl FnOnce(Stop) -> R + Send,
) -> R {
    let stopped = AtomicBool::new(stop());
    let stopped = &stopped;
    let flag = move || stopped.load(Ordering::SeqCst);
    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = scope.spawn(move || {
            let answer = work(&flag);
            let _ = sender.send(());
            answer
        });
        // Until it is done, or panicked (`join` says which).
        while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = receiver.recv_timeout(TICK) {
            if stop() {
                stopped.store(true, Ordering::SeqCst);
            }
        }
        match worker.join() {
            Ok(answer) => answer,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    #[test]
    fn permits_never_pass_the_cap_and_come_back() {
        let limiter = Limiter::new(3, 1);
        let running = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..12 {
                scope.spawn(|| {
                    let _permit = limiter.read(&never).unwrap();
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(10));
                    running.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        assert_eq!(peak.load(Ordering::SeqCst), 3);
        assert_eq!(limiter.reading(), 0);
        // RPC calls have their own permits.
        let _reads: Vec<_> = (0..3).map(|_| limiter.read(&never).unwrap()).collect();
        assert!(limiter.rpc(&never).is_ok());
    }

    #[test]
    fn a_wait_for_a_permit_hears_the_stop_promptly() {
        let limiter = Limiter::new(1, 1);
        let _held = limiter.read(&never).unwrap();
        let started = Instant::now();
        let stop = move || started.elapsed() > Duration::from_millis(50);
        assert_eq!(limiter.read(&stop).err().unwrap(), CANCELLED);
        assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
        // A stopped operation starts no read even with a permit free.
        let free = Limiter::new(1, 1);
        assert_eq!(free.read(&|| true).err().unwrap(), CANCELLED);
    }

    #[test]
    fn callers_of_one_key_at_once_share_one_read() {
        let limiter = Limiter::default();
        let reads = AtomicUsize::new(0);
        let waiting = std::sync::Barrier::new(4);
        let answers: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        waiting.wait();
                        limiter.shared("bafyX", &never, || {
                            reads.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(100));
                            Ok(b"x".to_vec())
                        })
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert!(answers.iter().all(|answer| answer.as_deref() == Ok(b"x".as_slice())));
        // Once landed, the next caller reads again.
        limiter.shared("bafyX", &never, || Ok(Vec::new())).unwrap();
        assert!(lock(&limiter.flights).is_empty());
    }

    /// A read nobody joined is handed back as it was read: the same
    /// allocation, never copied (a part can be hundreds of megabytes).
    #[test]
    fn a_read_nobody_shares_is_not_copied() {
        let limiter = Limiter::default();
        let read = Mutex::new(0usize);
        let answer = limiter
            .shared("bafyBig", &never, || {
                let bytes = vec![7u8; 1 << 20];
                *lock(&read) = bytes.as_ptr() as usize;
                Ok(bytes)
            })
            .unwrap();
        assert_eq!(answer.as_ptr() as usize, *lock(&read));
        assert!(lock(&limiter.flights).is_empty());
    }

    /// The builds' shared byte budget: what fits is held, what does not
    /// waits unless its build has nothing on its way, and comes back.
    #[test]
    fn the_shared_byte_budget_holds_what_fits_or_a_lone_part() {
        let limiter = Limiter::default().with_held(100);
        assert!(limiter.hold(60, false));
        assert!(!limiter.hold(50, false), "past the budget");
        assert_eq!(limiter.holding(), 60);
        assert!(limiter.hold(50, true), "a build with nothing on its way");
        assert_eq!(limiter.holding(), 110);
        limiter.let_go(110);
        assert_eq!(limiter.holding(), 0);
        assert!(limiter.hold(100, false));
    }

    #[test]
    fn a_shared_failure_reaches_every_caller_but_a_cancel_reads_again() {
        let limiter = Limiter::default();
        let reads = AtomicUsize::new(0);
        let (go, wait_go) = (std::sync::Barrier::new(2), std::sync::Barrier::new(2));
        std::thread::scope(|scope| {
            let leader = scope.spawn(|| {
                limiter.shared("bafyY", &never, || {
                    reads.fetch_add(1, Ordering::SeqCst);
                    go.wait();
                    wait_go.wait();
                    Err(CANCELLED.to_string())
                })
            });
            go.wait();
            let follower = scope.spawn(|| {
                limiter.shared("bafyY", &never, || {
                    reads.fetch_add(1, Ordering::SeqCst);
                    Err("Arbesk's storage has nothing at bafyY.".to_string())
                })
            });
            std::thread::sleep(Duration::from_millis(50));
            wait_go.wait();
            assert_eq!(leader.join().unwrap().unwrap_err(), CANCELLED);
            // The leader's cancel was its own: the follower read for itself.
            assert_eq!(
                follower.join().unwrap().unwrap_err(),
                "Arbesk's storage has nothing at bafyY."
            );
        });
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_follower_hears_its_own_stop() {
        let limiter = Limiter::default();
        let started = std::sync::Barrier::new(2);
        let release = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let leader = scope.spawn(|| {
                limiter.shared("bafyZ", &never, || {
                    started.wait();
                    release.wait();
                    Ok(b"z".to_vec())
                })
            });
            started.wait();
            let begun = Instant::now();
            let stop = move || begun.elapsed() > Duration::from_millis(50);
            let answer = limiter.shared("bafyZ", &stop, || panic!("the follower waits"));
            assert_eq!(answer.unwrap_err(), CANCELLED);
            assert!(begun.elapsed() < Duration::from_secs(1));
            release.wait();
            assert_eq!(leader.join().unwrap().unwrap(), b"z");
        });
    }

    #[test]
    fn a_bridged_stop_reaches_the_work() {
        let asked = std::cell::Cell::new(0);
        let stop = || {
            asked.set(asked.get() + 1);
            asked.get() > 2
        };
        let started = Instant::now();
        let seen = bridged(&stop, |stop| {
            while !stop() {
                std::thread::sleep(Duration::from_millis(10));
            }
            true
        });
        assert!(seen);
        assert!(started.elapsed() < Duration::from_secs(2));
        // Work that ends at once answers at once.
        assert_eq!(bridged(&|| false, |_| 7), 7);
    }
}

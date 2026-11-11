//! A short memo of `tokenURI` (performance plan, phase 4): the collection
//! manifest CID each (chain, contract, token) answered, kept for [`KEPT`]
//! so the calls of one operation (a scene's links, the copy-out plan's
//! assets, a preview's scene, saves and files) share one chain read.
//!
//! Only reads use it, through the [`crate::limiter::Limiter`] their `Http`
//! carries ([`crate::limiter::Limited`]); a write reads through the bare
//! client and never sees it: its compare-and-swap still reads `tokenURI`
//! under the collection hold. The memo is cleared by this app's own writes
//! and by every listing (Reload is one), and a read that started before a
//! clear is never kept after it.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::chain::ChainConfig;
use crate::limiter::Limits;

/// How long an answer is kept: long enough for one operation's calls
/// (a preview's scene, saves and files; a copy-out plan of many assets,
/// asked a few at a time) to share it, short enough that a save made
/// elsewhere (the Studio, another app) shows within a few seconds even
/// without a Reload, which clears it at once.
pub const KEPT: Duration = Duration::from_secs(5);
/// The most answers kept: a scene links to a handful of collections, an
/// account holds a few dozen; past this the oldest goes first.
pub const MAX_TOKENS: usize = 256;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The kept answers, and how many times they were cleared.
#[derive(Default)]
struct Kept {
    /// Goes up at each clear: an answer read before one is not kept.
    generation: u64,
    /// Each key's CID and when the read that answered it started.
    cids: HashMap<String, (String, Instant)>,
}

/// `tokenURI` answers, kept for a few seconds. One per [`crate::Client`].
pub struct TokenMemo {
    kept: Mutex<Kept>,
    /// How long an answer is kept ([`KEPT`]; tests use shorter).
    lasts: Duration,
    cap: usize,
}

impl Default for TokenMemo {
    fn default() -> Self {
        Self::new(KEPT, MAX_TOKENS)
    }
}

/// A token's key: the chain's RPC, the contract and the token id.
fn key(chain: &ChainConfig, token_id: &str) -> String {
    format!("{}\n{}\n{token_id}", chain.rpc_url, chain.contract)
}

impl TokenMemo {
    pub fn new(lasts: Duration, cap: usize) -> Self {
        Self {
            kept: Mutex::new(Kept::default()),
            lasts,
            cap: cap.max(1),
        }
    }

    /// The CID `token_id` of `chain` points at: kept from a read less than
    /// [`KEPT`] ago, else `read` (one `tokenURI` call), which callers
    /// asking for the same token at once share (`Limiter::shared`, waits
    /// hearing `limits.stop`). A failure is never kept.
    pub fn cid(
        &self,
        limits: Limits,
        chain: &ChainConfig,
        token_id: &str,
        read: impl Fn() -> Result<String, String>,
    ) -> Result<String, String> {
        let key = key(chain, token_id);
        let started = Instant::now();
        let generation = {
            let kept = lock(&self.kept);
            if let Some((cid, at)) = kept.cids.get(&key) {
                if at.elapsed() < self.lasts {
                    return Ok(cid.clone());
                }
            }
            kept.generation
        };
        // Callers after a clear never join a read from before it.
        let flight = format!("tokenURI\n{generation}\n{key}");
        let bytes = limits
            .limiter
            .shared(&flight, limits.stop, || read().map(String::into_bytes))?;
        let cid = String::from_utf8(bytes).map_err(|_| "The chain sent an answer File Browser can't read.".to_string())?;
        let mut kept = lock(&self.kept);
        if kept.generation == generation {
            self.make_room(&mut kept);
            kept.cids.insert(key, (cid.clone(), started));
        }
        Ok(cid)
    }

    /// Room for one more answer: the stale ones go, then the oldest.
    fn make_room(&self, kept: &mut Kept) {
        if kept.cids.len() < self.cap {
            return;
        }
        let lasts = self.lasts;
        kept.cids.retain(|_, (_, at)| at.elapsed() < lasts);
        while kept.cids.len() >= self.cap {
            let oldest = kept
                .cids
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(key, _)| key.clone());
            match oldest {
                Some(key) => kept.cids.remove(&key),
                None => break,
            };
        }
    }

    /// Forget every answer: this app wrote, or a listing (Reload) reads afresh.
    pub fn clear(&self) {
        let mut kept = lock(&self.kept);
        kept.generation += 1;
        kept.cids.clear();
    }

    /// How many answers are kept (tests).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        lock(&self.kept).cids.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limiter::{Limiter, never};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn chain(contract: &str) -> ChainConfig {
        ChainConfig {
            contract: contract.into(),
            rpc_url: "https://rpc.test".into(),
            gateway: "https://gw.test/ipfs".into(),
        }
    }

    fn limits(limiter: &Limiter) -> Limits<'_> {
        Limits { limiter, stop: &never }
    }

    #[test]
    fn an_answer_is_kept_for_a_few_seconds_then_read_again() {
        let limiter = Limiter::default();
        let memo = TokenMemo::new(Duration::from_millis(60), 8);
        let reads = AtomicUsize::new(0);
        let read = || {
            reads.fetch_add(1, Ordering::SeqCst);
            Ok(format!("bafy{}", reads.load(Ordering::SeqCst)))
        };
        let c = chain("0xC0");
        assert_eq!(memo.cid(limits(&limiter), &c, "1", read).unwrap(), "bafy1");
        assert_eq!(memo.cid(limits(&limiter), &c, "1", read).unwrap(), "bafy1");
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(memo.cid(limits(&limiter), &c, "1", read).unwrap(), "bafy2");
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn each_contract_and_token_is_its_own() {
        let limiter = Limiter::default();
        let memo = TokenMemo::default();
        let reads = AtomicUsize::new(0);
        let read = |cid: &'static str| {
            let reads = &reads;
            move || {
                reads.fetch_add(1, Ordering::SeqCst);
                Ok(cid.to_string())
            }
        };
        assert_eq!(memo.cid(limits(&limiter), &chain("0xC0"), "1", read("bafyA")).unwrap(), "bafyA");
        assert_eq!(memo.cid(limits(&limiter), &chain("0xC1"), "1", read("bafyB")).unwrap(), "bafyB");
        assert_eq!(memo.cid(limits(&limiter), &chain("0xC0"), "2", read("bafyC")).unwrap(), "bafyC");
        assert_eq!(memo.cid(limits(&limiter), &chain("0xC0"), "1", read("bafyX")).unwrap(), "bafyA");
        assert_eq!(reads.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn a_clear_forgets_and_a_failure_is_never_kept() {
        let limiter = Limiter::default();
        let memo = TokenMemo::default();
        let c = chain("0xC0");
        assert!(memo.cid(limits(&limiter), &c, "1", || Err("down".into())).is_err());
        assert_eq!(memo.len(), 0);
        assert_eq!(memo.cid(limits(&limiter), &c, "1", || Ok("bafyOld".into())).unwrap(), "bafyOld");
        memo.clear();
        assert_eq!(memo.cid(limits(&limiter), &c, "1", || Ok("bafyNew".into())).unwrap(), "bafyNew");
    }

    /// A read under way when the memo is cleared (a write landed) answers
    /// its own caller but is not kept, and nobody after the clear joins it.
    #[test]
    fn a_read_from_before_a_clear_is_not_kept() {
        let limiter = Limiter::default();
        let memo = TokenMemo::default();
        let c = chain("0xC0");
        let (started, start) = std::sync::mpsc::channel();
        let (finish, finished) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            let (memo, limiter, c) = (&memo, &limiter, &c);
            let early = scope.spawn(move || {
                memo.cid(limits(limiter), c, "1", || {
                    started.send(()).unwrap();
                    finished.recv().unwrap();
                    Ok("bafyBefore".into())
                })
            });
            start.recv().unwrap();
            memo.clear();
            assert_eq!(memo.cid(limits(limiter), c, "1", || Ok("bafyAfter".into())).unwrap(), "bafyAfter");
            finish.send(()).unwrap();
            assert_eq!(early.join().unwrap().unwrap(), "bafyBefore");
        });
        assert_eq!(memo.cid(limits(&limiter), &c, "1", || Ok("bafyX".into())).unwrap(), "bafyAfter");
    }

    #[test]
    fn callers_of_one_token_at_once_share_one_read() {
        let limiter = Limiter::default();
        let memo = TokenMemo::default();
        let c = chain("0xC0");
        let reads = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let callers: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        memo.cid(limits(&limiter), &c, "1", || {
                            reads.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(50));
                            Ok("bafyC".into())
                        })
                    })
                })
                .collect();
            for caller in callers {
                assert_eq!(caller.join().unwrap().unwrap(), "bafyC");
            }
        });
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_memo_never_holds_more_than_its_cap() {
        let limiter = Limiter::default();
        let memo = TokenMemo::new(KEPT, 4);
        let c = chain("0xC0");
        for token in 0..10 {
            let cid = format!("bafy{token}");
            memo.cid(limits(&limiter), &c, &token.to_string(), || Ok(cid.clone())).unwrap();
            assert!(memo.len() <= 4, "{}", memo.len());
        }
        // The newest stay; the oldest went first.
        let read = AtomicUsize::new(0);
        let again = |token: &str| {
            memo.cid(limits(&limiter), &c, token, || {
                read.fetch_add(1, Ordering::SeqCst);
                Ok("bafyRead".into())
            })
            .unwrap()
        };
        assert_eq!(again("9"), "bafy9");
        assert_eq!(read.load(Ordering::SeqCst), 0);
        assert_eq!(again("0"), "bafyRead");
    }
}

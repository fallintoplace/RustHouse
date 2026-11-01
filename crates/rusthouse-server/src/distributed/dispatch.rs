//! A fixed number of workers service all configured peers without wave barriers.
use crate::cluster::Peer;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Run all peers at bounded concurrency, retaining the configured peer order.
///
/// Claim indices dynamically to avoid head-of-line blocking between fixed
/// batches. Spawn failure falls back to fewer workers or inline execution.
/// Every peer gets one result slot, including failures and panics.
#[cfg(test)]
pub(super) fn map_peers_bounded<T: Send + Sync>(
    peers: &[Peer],
    max_workers: usize,
    work: impl Fn(Peer) -> Result<T, String> + Sync,
) -> Vec<Result<T, String>> {
    map_peers_bounded_impl(peers, max_workers, false, work)
}

/// Abort unclaimed peer requests once one shard has failed. Already running
/// requests finish, and the caller still receives a result slot per peer.
pub(super) fn map_peers_bounded_failfast<T: Send + Sync>(
    peers: &[Peer],
    max_workers: usize,
    work: impl Fn(Peer) -> Result<T, String> + Sync,
) -> Vec<Result<T, String>> {
    map_peers_bounded_impl(peers, max_workers, true, work)
}

fn map_peers_bounded_impl<T: Send + Sync>(
    peers: &[Peer],
    max_workers: usize,
    fail_fast: bool,
    work: impl Fn(Peer) -> Result<T, String> + Sync,
) -> Vec<Result<T, String>> {
    let cursor = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let slots: Vec<OnceLock<Result<T, String>>> =
        (0..peers.len()).map(|_| OnceLock::new()).collect();
    let process = || loop {
        let index = cursor.fetch_add(1, Ordering::Relaxed);
        if index >= peers.len() || (fail_fast && cancelled.load(Ordering::Acquire)) {
            break;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(peers[index])))
            .unwrap_or_else(|_| Err("peer query panicked".into()));
        if fail_fast && result.is_err() {
            cancelled.store(true, Ordering::Release);
        }
        // A claimed index belongs to exactly one worker.
        let _ = slots[index].set(result);
    };
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(max_workers.min(peers.len()));
        for _ in 0..max_workers.min(peers.len()) {
            match std::thread::Builder::new().spawn_scoped(scope, &process) {
                Ok(handle) => handles.push(handle),
                Err(_) => break,
            }
        }
        if handles.is_empty() {
            process();
        }
        for handle in handles {
            let _ = handle.join();
        }
    });
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner().unwrap_or_else(|| {
                Err(if fail_fast && cancelled.load(Ordering::Acquire) {
                    "peer query cancelled after another shard failed".into()
                } else {
                    "peer query panicked".into()
                })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn visits_every_peer_once_and_preserves_order_under_contention() {
        let peers = (1..=32)
            .map(|port| Peer::parse(&format!("127.0.0.1:{}", 42000 + port)).unwrap())
            .collect::<Vec<_>>();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let calls = AtomicUsize::new(0);
        let results = map_peers_bounded(&peers, 4, |peer| {
            calls.fetch_add(1, Ordering::SeqCst);
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            if peer.port() % 2 == 0 {
                std::thread::yield_now();
            }
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(peer.port())
        });
        assert_eq!(calls.load(Ordering::SeqCst), peers.len());
        assert!(peak.load(Ordering::SeqCst) <= 4);
        assert_eq!(
            results.into_iter().collect::<Result<Vec<_>, _>>().unwrap(),
            peers.iter().map(|peer| peer.port()).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn runs_inline_when_no_workers_requested_and_reports_panics() {
        let peers = [
            Peer::parse("127.0.0.1:43001").unwrap(),
            Peer::parse("127.0.0.1:43002").unwrap(),
        ];
        let results = map_peers_bounded(&peers, 0, |peer| {
            if peer.port() == 43001 {
                panic!("simulated peer panic")
            }
            Ok(peer.port())
        });
        assert_eq!(results[0], Err("peer query panicked".to_owned()));
        assert_eq!(results[1], Ok(43002));
    }
}

#[cfg(test)]
mod fail_fast_fanout_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn avoids_dispatching_more_peers_after_a_failed_shard() {
        let peers = (0..32)
            .map(|index| Peer::parse(&format!("127.0.0.1:{}", 45000 + index)).unwrap())
            .collect::<Vec<_>>();
        let attempts = AtomicUsize::new(0);
        let results = map_peers_bounded_failfast(&peers, 1, |peer| {
            attempts.fetch_add(1, Ordering::SeqCst);
            if peer.port() == 45000 {
                Err("first shard unavailable".into())
            } else {
                Ok(peer.port())
            }
        });
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(results.len(), 32);
        assert_eq!(results[0], Err("first shard unavailable".into()));
        assert!(
            results[1..]
                .iter()
                .all(|result| result
                    == &Err("peer query cancelled after another shard failed".into()))
        );
    }

    #[test]
    fn successful_failfast_fanout_matches_unbounded_work_order() {
        let peers = (0..32)
            .map(|index| Peer::parse(&format!("127.0.0.1:{}", 45100 + index)).unwrap())
            .collect::<Vec<_>>();
        let attempts = AtomicUsize::new(0);
        let results = map_peers_bounded_failfast(&peers, 4, |peer| {
            attempts.fetch_add(1, Ordering::SeqCst);
            Ok(peer.port())
        });
        assert_eq!(attempts.load(Ordering::SeqCst), peers.len());
        assert_eq!(
            results.into_iter().collect::<Result<Vec<_>, _>>().unwrap(),
            peers.iter().map(|peer| peer.port()).collect::<Vec<_>>(),
        );
    }
}

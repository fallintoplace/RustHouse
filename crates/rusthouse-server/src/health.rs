//! On-demand health probes for explicitly configured loopback peers.

use crate::Response;
use crate::cluster::{Cluster, Peer};
use crate::remote;
use std::fmt::Write as _;

fn is_peer_healthy(peer: Peer) -> bool {
    matches!(
        remote::ping(peer),
        Ok(reply) if reply.status == 200 && reply.body == b"Ok.\n"
    )
}

const MAX_INFLIGHT_HEALTH_PROBES: usize = 8;

fn probe_peers_with(peers: &[Peer], ping: impl Fn(Peer) -> bool + Sync) -> Vec<bool> {
    probe_peers_with_workers(peers, MAX_INFLIGHT_HEALTH_PROBES, ping)
}

fn probe_peers_with_workers(
    peers: &[Peer],
    max_workers: usize,
    ping: impl Fn(Peer) -> bool + Sync,
) -> Vec<bool> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    // Use at most eight workers per request, not eight fresh threads for
    // every chunk. Dynamic claims avoid waiting for all slow peers in one
    // chunk before starting probes in the next one.
    // Most deployments have just one peer. Avoid a scoped-thread spawn
    // while preserving panic isolation for that peer.
    if let [peer] = peers {
        let healthy =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ping(*peer))).unwrap_or(false);
        return vec![healthy];
    }
    let cursor = AtomicUsize::new(0);
    let results = (0..peers.len())
        .map(|_| AtomicBool::new(false))
        .collect::<Vec<_>>();
    // Worker startup can fail under process thread limits. Service the
    // remaining peers with fewer workers, or inline if no thread starts.
    let process = || loop {
        let index = cursor.fetch_add(1, Ordering::Relaxed);
        if index >= peers.len() {
            break;
        }
        let healthy = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ping(peers[index])
        }))
        .unwrap_or(false);
        results[index].store(healthy, Ordering::Relaxed);
    };
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(peers.len().min(max_workers));
        for _ in 0..peers.len().min(max_workers) {
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
    results
        .iter()
        .map(|healthy| healthy.load(Ordering::Relaxed))
        .collect()
}

fn probe_peer_health(cluster: &Cluster) -> Vec<bool> {
    probe_peers_with(cluster.peers(), is_peer_healthy)
}

fn render_health_response(cluster: &Cluster, healthy: &[bool]) -> Response {
    // A missing probe result must never count as a healthy peer. This also
    // keeps the reported total aligned with the configured membership.
    let peers = cluster.peers();
    let complete = healthy.len() == peers.len();
    let healthy_peer_count = peers
        .iter()
        .enumerate()
        .filter(|(index, _)| healthy.get(*index) == Some(&true))
        .count();
    let status = if complete && !peers.is_empty() && healthy_peer_count == peers.len() {
        "200 OK"
    } else {
        "503 Service Unavailable"
    };
    let mut body = String::with_capacity(48 + peers.len() * 64);
    write!(
        &mut body,
        "{{\"total\":{},\"healthy\":{healthy_peer_count},\"peers\":[",
        peers.len()
    )
    .expect("writing to String cannot fail");
    for (index, peer) in peers.iter().enumerate() {
        let alive = healthy.get(index).copied().unwrap_or(false);
        if index != 0 {
            body.push(',');
        }
        write!(
            &mut body,
            "{{\"address\":\"{}\",\"healthy\":{alive}}}",
            peer.address()
        )
        .expect("writing to String cannot fail");
    }
    body.push_str("]}\n");
    Response {
        status,
        content_type: "application/json; charset=utf-8",
        body: body.into_bytes(),
    }
}

pub fn handle(method: &str, body: &[u8], cluster: &Cluster) -> Response {
    if method != "GET" {
        return Response::text("405 Method Not Allowed", "health endpoint requires GET\n");
    }
    if !body.is_empty() {
        return Response::text("400 Bad Request", "GET must not contain a request body\n");
    }
    let healthy = probe_peer_health(cluster);
    render_health_response(cluster, &healthy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::Peer;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    #[test]
    fn single_buffer_health_response_keeps_exact_peer_order() {
        let cluster = Cluster::new(vec![
            Peer::parse("127.0.0.1:45231").unwrap(),
            Peer::parse("127.0.0.1:45232").unwrap(),
        ])
        .unwrap();
        let response = render_health_response(&cluster, &[true, false]);
        assert_eq!(response.status, "503 Service Unavailable");
        assert_eq!(response.body, b"{\"total\":2,\"healthy\":1,\"peers\":[{\"address\":\"127.0.0.1:45231\",\"healthy\":true},{\"address\":\"127.0.0.1:45232\",\"healthy\":false}]}\n");
    }

    #[test]
    fn reports_empty_cluster_without_peer_allocations() {
        let cluster = Cluster::new(Vec::new()).unwrap();
        let response = handle("GET", b"", &cluster);
        assert_eq!(response.status, "503 Service Unavailable");
        assert_eq!(response.body, b"{\"total\":0,\"healthy\":0,\"peers\":[]}\n");
    }

    #[test]
    fn reports_healthy_peer_and_rejects_post() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let responder = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            // Drain the request before responding. Closing with unread TCP data
            // can reset the connection and hide an otherwise valid reply.
            let mut reader = BufReader::new(&mut socket);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            drop(reader);
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Type: text/plain\r\n\r\nOk.\n",
                )
                .unwrap();
        });
        let cluster =
            Cluster::new(vec![Peer::parse(&format!("127.0.0.1:{port}")).unwrap()]).unwrap();
        let response = handle("GET", b"", &cluster);
        assert_eq!(response.status, "200 OK");
        let body = String::from_utf8(response.body).unwrap();
        assert!(body.contains("\"healthy\":1"));
        assert!(body.contains("\"address\":"));
        assert!(body.contains("\"healthy\":true"));
        responder.join().unwrap();
        assert_eq!(
            handle("POST", b"", &cluster).status,
            "405 Method Not Allowed"
        );
    }

    #[test]
    fn reports_unreachable_peer_without_hanging() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let cluster =
            Cluster::new(vec![Peer::parse(&format!("127.0.0.1:{port}")).unwrap()]).unwrap();
        let response = handle("GET", b"", &cluster);
        assert_eq!(response.status, "503 Service Unavailable");
        assert!(response.body.ends_with(b"\"healthy\":false}]}\n"));
        assert_eq!(handle("GET", b"x", &cluster).status, "400 Bad Request");
    }
}

#[cfg(test)]
mod health_probe_budget_tests {
    use super::*;

    #[test]
    fn large_clusters_have_one_health_result_per_peer() {
        let peers = (0..crate::cluster::MAX_PEERS)
            .map(|index| Peer::parse(&format!("127.0.0.1:{}", 60000 + index)).unwrap())
            .collect();
        let cluster = Cluster::new(peers).unwrap();
        let results = probe_peer_health(&cluster);
        assert_eq!(results.len(), crate::cluster::MAX_PEERS);
        // Local ports are not guaranteed to be unused on CI runners.
        // This test checks that every registered peer has a result.
    }
}

#[cfg(test)]
mod dynamic_health_workers_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn bounded_workers_visit_every_peer_once_and_keep_input_order() {
        let peers = (0..crate::cluster::MAX_PEERS)
            .map(|index| Peer::parse(&format!("127.0.0.1:{}", 47000 + index)).unwrap())
            .collect::<Vec<_>>();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let calls = AtomicUsize::new(0);
        let health = probe_peers_with(&peers, |peer| {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(2));
            active.fetch_sub(1, Ordering::SeqCst);
            calls.fetch_add(1, Ordering::SeqCst);
            peer.address().port() % 2 == 0
        });
        assert_eq!(calls.load(Ordering::SeqCst), peers.len());
        assert!(peak.load(Ordering::SeqCst) <= MAX_INFLIGHT_HEALTH_PROBES);
        assert_eq!(
            health,
            peers
                .iter()
                .map(|peer| peer.address().port() % 2 == 0)
                .collect::<Vec<_>>()
        );
        assert!(probe_peers_with(&[], |_| true).is_empty());
    }
}

#[cfg(test)]
mod panicking_health_probe_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn failing_peer_does_not_skip_other_health_probes() {
        let peers = (0..crate::cluster::MAX_PEERS)
            .map(|index| Peer::parse(&format!("127.0.0.1:{}", 49000 + index)).unwrap())
            .collect::<Vec<_>>();
        let visits = (0..peers.len())
            .map(|_| AtomicUsize::new(0))
            .collect::<Vec<_>>();
        let result = probe_peers_with(&peers, |peer| {
            let index = usize::from(peer.port() - 49000);
            visits[index].fetch_add(1, Ordering::SeqCst);
            if index == 0 {
                panic!("one peer probe failed");
            }
            index % 2 == 0
        });
        assert_eq!(result.len(), peers.len());
        for (index, count) in visits.iter().enumerate() {
            assert_eq!(count.load(Ordering::SeqCst), 1, "probe {index} was skipped");
            assert_eq!(result[index], index != 0 && index % 2 == 0);
        }
    }
}

#[cfg(test)]
mod inline_health_probe_tests {
    use super::*;

    #[test]
    fn one_peer_is_checked_inline_and_panics_stay_unhealthy() {
        let owner = std::thread::current().id();
        let peers = [Peer::parse("127.0.0.1:50123").unwrap()];
        assert_eq!(
            probe_peers_with(&peers, |_| {
                assert_eq!(std::thread::current().id(), owner);
                true
            }),
            vec![true]
        );
        assert_eq!(
            probe_peers_with(&peers, |_| panic!("probe failed")),
            vec![false]
        );
    }
}

#[cfg(test)]
mod incomplete_health_results_tests {
    use super::*;

    #[test]
    fn missing_or_extra_results_never_mark_a_cluster_healthy() {
        let cluster = Cluster::new(vec![
            Peer::parse("127.0.0.1:59001").unwrap(),
            Peer::parse("127.0.0.1:59002").unwrap(),
        ])
        .unwrap();
        let missing = render_health_response(&cluster, &[true]);
        assert_eq!(missing.status, "503 Service Unavailable");
        assert_eq!(missing.body, b"{\"total\":2,\"healthy\":1,\"peers\":[{\"address\":\"127.0.0.1:59001\",\"healthy\":true},{\"address\":\"127.0.0.1:59002\",\"healthy\":false}]}\n");
        let extra = render_health_response(&cluster, &[true, true, true]);
        assert_eq!(extra.status, "503 Service Unavailable");
        assert_eq!(extra.body, b"{\"total\":2,\"healthy\":2,\"peers\":[{\"address\":\"127.0.0.1:59001\",\"healthy\":true},{\"address\":\"127.0.0.1:59002\",\"healthy\":true}]}\n");
        assert_eq!(
            render_health_response(&cluster, &[true, true]).status,
            "200 OK"
        );
    }
}

#[cfg(test)]
mod health_spawn_fallback_tests {
    use super::*;

    #[test]
    fn zero_available_workers_probe_all_peers_on_the_calling_thread() {
        let caller = std::thread::current().id();
        let peers = (0..12)
            .map(|index| Peer::parse(&format!("127.0.0.1:{}", 55000 + index)).unwrap())
            .collect::<Vec<_>>();
        let result = probe_peers_with_workers(&peers, 0, |peer| {
            assert_eq!(std::thread::current().id(), caller);
            peer.port() % 2 == 0
        });
        assert_eq!(
            result,
            peers.iter().map(|peer| peer.port() % 2 == 0).collect::<Vec<_>>()
        );
    }

    #[test]
    fn inline_fallback_isolates_panicking_probes() {
        let peers = (0..4)
            .map(|index| Peer::parse(&format!("127.0.0.1:{}", 55100 + index)).unwrap())
            .collect::<Vec<_>>();
        let result = probe_peers_with_workers(&peers, 0, |peer| {
            if peer.port() == 55101 {
                panic!("simulated failed probe");
            }
            true
        });
        assert_eq!(result, vec![true, false, true, true]);
    }
}

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

fn probe_peer_health(cluster: &Cluster) -> Vec<bool> {
    // Probe each peer in parallel, bounded by the 16-peer registration limit.
    std::thread::scope(|scope| {
        let handles = cluster
            .peers()
            .iter()
            .map(|&peer| scope.spawn(move || is_peer_healthy(peer)))
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or(false))
            .collect()
    })
}

fn render_peer_health_entries(cluster: &Cluster, healthy: &[bool]) -> String {
    let mut details = String::new();
    for (index, (peer, &alive)) in cluster.peers().iter().zip(healthy).enumerate() {
        if index != 0 {
            details.push(',');
        }
        write!(
            &mut details,
            "{{\"address\":\"{}\",\"healthy\":{alive}}}",
            peer.address()
        )
        .expect("writing to String cannot fail");
    }
    details
}

fn render_health_response(cluster: &Cluster, healthy: &[bool]) -> Response {
    let count = healthy.iter().filter(|&&alive| alive).count();
    let details = render_peer_health_entries(cluster, healthy);
    let status = if count == healthy.len() {
        "200 OK"
    } else {
        "503 Service Unavailable"
    };
    Response {
        status,
        content_type: "application/json; charset=utf-8",
        body: format!(
            "{{\"total\":{},\"healthy\":{count},\"peers\":[{details}]}}\n",
            healthy.len()
        )
        .into_bytes(),
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
    fn reports_empty_cluster_without_peer_allocations() {
        let cluster = Cluster::new(Vec::new()).unwrap();
        let response = handle("GET", b"", &cluster);
        assert_eq!(response.status, "200 OK");
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

//! On-demand health probes for explicitly configured loopback peers.

use crate::Response;
use crate::cluster::Cluster;
use crate::remote;

pub fn handle(method: &str, body: &[u8], cluster: &Cluster) -> Response {
    if method != "GET" {
        return Response::text("405 Method Not Allowed", "health endpoint requires GET\n");
    }
    if !body.is_empty() {
        return Response::text("400 Bad Request", "GET must not contain a request body\n");
    }
    // Probe each peer in parallel, bounded by the 16-peer registration limit.
    let healthy: Vec<bool> = std::thread::scope(|scope| {
        let handles = cluster
            .peers()
            .iter()
            .map(|&peer| {
                scope.spawn(move || {
                    matches!(
                        remote::ping(peer),
                        Ok(reply) if reply.status == 200 && reply.body == b"Ok.\n"
                    )
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or(false))
            .collect()
    });
    let count = healthy.iter().filter(|&&alive| alive).count();
    let details = cluster
        .peers()
        .iter()
        .zip(&healthy)
        .map(|(peer, &alive)| format!("{{\"address\":\"{}\",\"healthy\":{alive}}}", peer.address()))
        .collect::<Vec<_>>()
        .join(",");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::Peer;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

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

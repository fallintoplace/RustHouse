//! Explicit localhost peer membership for an experimental read-only coordinator.
//! No network discovery, DNS, remote hosts or implicit local data sharding.

use crate::Response;
use std::net::{Ipv4Addr, SocketAddrV4};

pub const MAX_PEERS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    port: u16,
}

impl Peer {
    pub fn parse(address: &str) -> Result<Self, &'static str> {
        let port = address
            .strip_prefix("127.0.0.1:")
            .ok_or("peers must use 127.0.0.1:<port>")?;
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err("invalid peer port");
        }
        let port = port.parse::<u16>().map_err(|_| "invalid peer port")?;
        if port == 0 {
            return Err("peer port must be nonzero");
        }
        Ok(Self { port })
    }

    pub fn port(self) -> u16 {
        self.port
    }

    pub fn address(self) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.port)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Cluster {
    peers: Vec<Peer>,
}

impl Cluster {
    pub fn new(peers: Vec<Peer>) -> Result<Self, &'static str> {
        if peers.len() > MAX_PEERS {
            return Err("too many peers (maximum 16)");
        }
        for (index, peer) in peers.iter().enumerate() {
            if peers[..index].contains(peer) {
                return Err("duplicate peer");
            }
        }
        Ok(Self { peers })
    }

    pub fn peers(&self) -> &[Peer] {
        &self.peers
    }
}

/// Membership describes peer addresses, not their health or consistency.
pub fn handle(method: &str, body: &[u8], cluster: &Cluster) -> Response {
    if method != "GET" {
        return Response::text("405 Method Not Allowed", "cluster endpoint requires GET\n");
    }
    if !body.is_empty() {
        return Response::text("400 Bad Request", "GET must not contain a request body\n");
    }
    let addresses = cluster
        .peers()
        .iter()
        .map(|peer| format!("\"{}\"", peer.address()))
        .collect::<Vec<_>>()
        .join(",");
    Response {
        status: "200 OK",
        content_type: "application/json; charset=utf-8",
        body: format!(
            "{{\"mode\":\"read-only\",\"peer_count\":{},\"peers\":[{addresses}]}}\n",
            cluster.peers().len()
        )
        .into_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_canonical_ipv4_loopback_peers() {
        assert_eq!(Peer::parse("127.0.0.1:8124").unwrap().port(), 8124);
        for input in [
            "", "localhost:8124", "0.0.0.0:8124", "127.0.0.2:8124",
            "8.8.8.8:8124", "127.0.0.1:0", "127.0.0.1:65536",
            "127.0.0.1:-1", "127.0.0.1:8123/path",
        ] {
            assert!(Peer::parse(input).is_err(), "{input}");
        }
    }

    #[test]
    fn rejects_duplicates_and_excessive_membership() {
        let peer = Peer::parse("127.0.0.1:8124").unwrap();
        assert!(Cluster::new(vec![peer, peer]).is_err());
        let peers = (1..=MAX_PEERS + 1)
            .map(|port| Peer { port: port as u16 })
            .collect();
        assert!(Cluster::new(peers).is_err());
    }

    #[test]
    fn lists_only_explicit_peers_and_rejects_write_methods() {
        let cluster = Cluster::new(vec![Peer::parse("127.0.0.1:8124").unwrap()]).unwrap();
        let response = handle("GET", b"", &cluster);
        assert_eq!(response.status, "200 OK");
        assert_eq!(
            response.body,
            b"{\"mode\":\"read-only\",\"peer_count\":1,\"peers\":[\"127.0.0.1:8124\"]}\n"
        );
        assert_eq!(handle("POST", b"", &cluster).status, "405 Method Not Allowed");
        assert_eq!(handle("GET", b"x", &cluster).status, "400 Bad Request");
    }
}

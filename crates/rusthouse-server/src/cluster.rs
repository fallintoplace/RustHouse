//! Explicit IPv4 peer membership for an experimental read-only coordinator.
//! Private network transport is opt-in and unauthenticated.

use crate::Response;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::net::{Ipv4Addr, SocketAddrV4};

pub const MAX_PEERS: usize = 32;

/// Accept only the private IPv4 ranges used by LANs and overlay networks.
pub fn is_private_address(address: Ipv4Addr) -> bool {
    match address.octets() {
        [10, ..] => true,
        [172, second, ..] if (16..=31).contains(&second) => true,
        [192, 168, ..] => true,
        [100, second, ..] if (64..=127).contains(&second) => true,
        _ => false,
    }
}

fn permitted_peer(address: Ipv4Addr, allow_private_network: bool) -> bool {
    address == Ipv4Addr::LOCALHOST || (allow_private_network && is_private_address(address))
}

fn validate_peer_address(
    address: &SocketAddrV4,
    allow_private_network: bool,
) -> Result<(), &'static str> {
    if address.port() == 0 {
        return Err("peer port must be nonzero");
    }
    if !permitted_peer(*address.ip(), allow_private_network) {
        return Err("remote peers require --allow-private-network and private IPv4");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    address: SocketAddrV4,
}

impl Peer {
    pub fn parse(address: &str) -> Result<Self, &'static str> {
        Self::parse_with_private_network(address, false)
    }

    pub fn parse_with_private_network(
        address: &str,
        allow_private_network: bool,
    ) -> Result<Self, &'static str> {
        let address: SocketAddrV4 = address
            .parse()
            .map_err(|_| "peers must use a numeric IPv4 address and port")?;
        validate_peer_address(&address, allow_private_network)?;
        Ok(Self { address })
    }

    pub fn port(self) -> u16 {
        self.address.port()
    }

    pub fn address(self) -> SocketAddrV4 {
        self.address
    }
}

#[derive(Clone, Debug, Default)]
pub struct Cluster {
    peers: Vec<Peer>,
}

impl Cluster {
    pub fn new(peers: Vec<Peer>) -> Result<Self, &'static str> {
        if peers.len() > MAX_PEERS {
            return Err("too many peers (maximum 32)");
        }
        let mut seen = HashSet::with_capacity(peers.len());
        for peer in &peers {
            if !seen.insert(peer.address()) {
                return Err("duplicate peer");
            }
        }
        Ok(Self { peers })
    }

    pub fn peers(&self) -> &[Peer] {
        &self.peers
    }
}

fn render_peer_addresses(peers: &[Peer]) -> String {
    let mut addresses = String::new();
    for (index, peer) in peers.iter().enumerate() {
        if index != 0 {
            addresses.push(',');
        }
        write!(&mut addresses, "\"{}\"", peer.address()).expect("writing to String cannot fail");
    }
    addresses
}

/// Membership describes peer addresses, not their health or consistency.
pub fn handle(method: &str, body: &[u8], cluster: &Cluster) -> Response {
    if method != "GET" {
        return Response::text("405 Method Not Allowed", "cluster endpoint requires GET\n");
    }
    if !body.is_empty() {
        return Response::text("400 Bad Request", "GET must not contain a request body\n");
    }
    let addresses = render_peer_addresses(cluster.peers());
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
            "",
            "localhost:8124",
            "0.0.0.0:8124",
            "127.0.0.2:8124",
            "8.8.8.8:8124",
            "127.0.0.1:0",
            "127.0.0.1:65536",
            "127.0.0.1:-1",
            "127.0.0.1:8123/path",
        ] {
            assert!(Peer::parse(input).is_err(), "{input}");
        }
    }

    #[test]
    fn private_peers_need_explicit_opt_in() {
        for address in [
            "10.4.0.1:8124",
            "172.16.0.1:8124",
            "192.168.1.2:8124",
            "100.101.102.103:8124",
        ] {
            assert!(Peer::parse(address).is_err());
            assert!(Peer::parse_with_private_network(address, true).is_ok());
        }
        for address in [
            "8.8.8.8:8124",
            "0.0.0.0:8124",
            "127.0.0.2:8124",
            "[::1]:8124",
            "169.254.1.1:8124",
        ] {
            assert!(
                Peer::parse_with_private_network(address, true).is_err(),
                "{address}"
            );
        }
    }

    #[test]
    fn supports_thirty_two_explicit_peers() {
        let peers = (1..=MAX_PEERS)
            .map(|port| Peer::parse(&format!("127.0.0.1:{port}")).unwrap())
            .collect();
        let cluster = Cluster::new(peers).unwrap();
        assert_eq!(cluster.peers().len(), 32);
    }

    #[test]
    fn rejects_duplicates_and_excessive_membership() {
        let peer = Peer::parse("127.0.0.1:8124").unwrap();
        assert!(Cluster::new(vec![peer, peer]).is_err());
        let peers = (1..=MAX_PEERS + 1)
            .map(|port| Peer::parse(&format!("127.0.0.1:{port}")).unwrap())
            .collect();
        assert!(Cluster::new(peers).is_err());
    }

    #[test]
    fn lists_multiple_peers_without_extra_separators() {
        let cluster = Cluster::new(vec![
            Peer::parse("127.0.0.1:8124").unwrap(),
            Peer::parse("127.0.0.1:8125").unwrap(),
        ])
        .unwrap();
        assert_eq!(
            handle("GET", b"", &cluster).body,
            b"{\"mode\":\"read-only\",\"peer_count\":2,\"peers\":[\"127.0.0.1:8124\",\"127.0.0.1:8125\"]}\n"
        );
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
        assert_eq!(
            handle("POST", b"", &cluster).status,
            "405 Method Not Allowed"
        );
        assert_eq!(handle("GET", b"x", &cluster).status, "400 Bad Request");
    }
}

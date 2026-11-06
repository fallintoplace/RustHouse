use super::*;

#[test]
fn accepts_only_bound_address_or_loopback_alias() {
    let loopback: SocketAddr = "127.0.0.1:8123".parse().unwrap();
    for valid in ["127.0.0.1", "127.0.0.1:8123", "localhost", "LOCALHOST:8123"] {
        assert!(host_matches_listener(valid, loopback), "{valid}");
    }
    for bad in [
        "attacker.example",
        "attacker.example:8123",
        "127.0.0.1:8124",
        "localhost:8124",
        "127.0.0.1:8123@evil.example",
        "",
    ] {
        assert!(!host_matches_listener(bad, loopback), "{bad}");
    }
    let private: SocketAddr = "10.1.2.3:8080".parse().unwrap();
    assert!(host_matches_listener("10.1.2.3:8080", private));
    assert!(!host_matches_listener("localhost:8080", private));
    assert!(!host_matches_listener("10.1.2.4:8080", private));
}

#[test]
fn accepts_bracketed_ipv6_without_disabling_host_validation() {
    let listener: SocketAddr = "[::1]:8123".parse().unwrap();
    for host in ["[::1]", "[::1]:8123", "[0:0:0:0:0:0:0:1]:8123"] {
        assert!(host_matches_listener(host, listener), "{host}");
    }
    for host in [
        "::1",
        "::1:8123",
        "[::1]:8124",
        "[::2]:8123",
        "[::1]garbage",
        "[::1]:",
        "[::1]:1234:8123",
        "[localhost]:8123",
        "[::1",
        "[::1]]:8123",
    ] {
        assert!(!host_matches_listener(host, listener), "{host}");
    }
    let v4: SocketAddr = "127.0.0.1:8123".parse().unwrap();
    assert!(!host_matches_listener("[::1]:8123", v4));
    assert!(host_matches_listener("localhost:8123", listener));
}

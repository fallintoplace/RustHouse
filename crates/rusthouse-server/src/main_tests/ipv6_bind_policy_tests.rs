use super::*;

#[test]
fn enables_ipv6_loopback_only_without_expanding_private_network_access() {
    for allow_private in [false, true] {
        assert!(permitted_listener_bind(
            "127.0.0.1".parse().unwrap(),
            allow_private
        ));
        assert!(permitted_listener_bind(
            "::1".parse().unwrap(),
            allow_private
        ));
        assert!(!permitted_listener_bind(
            "::".parse().unwrap(),
            allow_private
        ));
        assert!(!permitted_listener_bind(
            "fc00::1".parse().unwrap(),
            allow_private
        ));
        assert!(!permitted_listener_bind(
            "fe80::1".parse().unwrap(),
            allow_private
        ));
        assert!(!permitted_listener_bind(
            "8.8.8.8".parse().unwrap(),
            allow_private
        ));
    }
    assert!(!permitted_listener_bind("10.1.2.3".parse().unwrap(), false));
    assert!(permitted_listener_bind("10.1.2.3".parse().unwrap(), true));
}

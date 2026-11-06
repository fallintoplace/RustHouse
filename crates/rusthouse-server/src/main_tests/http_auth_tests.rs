use super::*;

#[test]
fn checks_exact_bearer_token_without_prefix_acceptance() {
    let secret = "0123456789abcdef";
    assert!(valid_configured_token(secret));
    assert!(bearer_token_matches(
        secret,
        Some("Bearer 0123456789abcdef")
    ));
    for supplied in [
        None,
        Some(""),
        Some("Basic 0123456789abcdef"),
        Some("Bearer 0123456789abcde"),
        Some("Bearer 0123456789abcdefextra"),
        Some("bearer 0123456789abcdef"),
        Some("Bearer 0123456789abcdeg"),
    ] {
        assert!(!bearer_token_matches(secret, supplied));
    }
    assert!(!valid_configured_token("short"));
    assert!(!valid_configured_token("has spaces invalid"));
}

use super::retryable_accept_error;
use std::io;

#[test]
fn only_expected_transient_accept_errors_are_retryable() {
    for kind in [io::ErrorKind::Interrupted, io::ErrorKind::ConnectionAborted] {
        assert!(retryable_accept_error(kind));
    }
    for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::Other] {
        assert!(!retryable_accept_error(kind));
    }
}

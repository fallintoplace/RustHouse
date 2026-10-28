//! Public HTTP contract checks. These tests intentionally call the server's
//! exposed API, not private helpers, to catch regressions affecting clients.
use rusthouse_server::{Config, handle_request};

fn config() -> Config {
    Config { part: None, database: None, max_rows: 100 }
}

#[test]
fn capability_discovery_has_stable_read_only_fields() {
    let response = handle_request("GET", "/capabilities", b"", &config());
    assert_eq!(response.status, "200 OK");
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let text = String::from_utf8(response.body).unwrap();
    assert!(text.contains("\"source\":\"generated\""));
    assert!(text.contains("\"read_only\":true"));
    assert!(text.contains("\"max_query_bytes\":"));
    assert!(text.contains("\"max_rows\":100"));
}

#[test]
fn json_numbers_are_lossless_strings() {
    let response = handle_request(
        "POST", "/?format=json", b"SELECT number FROM numbers(3)", &config(),
    );
    assert_eq!(response.status, "200 OK");
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let data = String::from_utf8(response.body).unwrap();
    assert!(data.contains("\"number\":\"0\""), "{data}");
    assert!(data.contains("\"number\":\"2\""), "{data}");
}

#[test]
fn unsupported_writes_and_malformed_json_formats_fail_closed() {
    for sql in ["CREATE TABLE x (n UInt64)", "DROP TABLE x", "INSERT INTO x VALUES (1)"] {
        let response = handle_request("POST", "/?format=json", sql.as_bytes(), &config());
        assert_ne!(response.status, "200 OK", "accepted mutating SQL: {sql}");
    }
    let response = handle_request("POST", "/?format=unknown", b"SELECT 1", &config());
    assert_eq!(response.status, "400 Bad Request");
}

#[test]
fn discovery_endpoints_reject_request_bodies() {
    for path in ["/ping", "/capabilities", "/schema"] {
        let response = handle_request("GET", path, b"unexpected", &config());
        assert_eq!(response.status, "400 Bad Request", "{path}");
    }
}

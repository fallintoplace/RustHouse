//! Public HTTP contract checks. These tests intentionally call the server's
//! exposed API, not private helpers, to catch regressions affecting clients.
use rusthouse_server::{Config, handle_request};

fn config() -> Config {
    Config {
        part: None,
        database: None,
        max_rows: 100,
    }
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
        "POST",
        "/?format=json",
        b"SELECT number FROM numbers(3)",
        &config(),
    );
    assert_eq!(response.status, "200 OK");
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    let data = String::from_utf8(response.body).unwrap();
    assert!(data.contains("\"number\":\"0\""), "{data}");
    assert!(data.contains("\"number\":\"2\""), "{data}");
}

#[test]
fn unsupported_writes_and_malformed_json_formats_fail_closed() {
    for sql in [
        "CREATE TABLE x (n UInt64)",
        "DROP TABLE x",
        "INSERT INTO x VALUES (1)",
    ] {
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

#[test]
fn clickhouse_http_post_can_split_sql_across_url_and_body() {
    let url_only = handle_request("POST", "/?query=SELECT+7", b"", &config());
    assert_eq!(url_only.status, "200 OK");
    assert_eq!(url_only.body, b"7\n7\n");

    let body_only = handle_request("POST", "/?query=", b"SELECT 8", &config());
    assert_eq!(body_only.status, "200 OK");
    assert_eq!(body_only.body, b"8\n8\n");

    let split = handle_request("POST", "/?query=SELECT", b"number FROM numbers(3)", &config());
    assert_eq!(split.status, "200 OK");
    assert_eq!(split.body, b"number\n0\n1\n2\n");

    let split_filter = handle_request(
        "POST",
        "/?query=SELECT+number+FROM+numbers(5)+WHERE",
        b"number > 2 ORDER BY number DESC LIMIT 2",
        &config(),
    );
    assert_eq!(split_filter.status, "200 OK");
    assert_eq!(split_filter.body, b"number\n4\n3\n");
}

#[test]
fn clickhouse_http_post_split_sql_is_bounded_and_never_joins_tokens() {
    // ClickHouse inserts a line feed between the two pieces. The split must
    // not permit concatenating fragments into keywords or new statements.
    let bad_keyword = handle_request("POST", "/?query=SEL", b"ECT 1", &config());
    assert_eq!(bad_keyword.status, "400 Bad Request");

    let mutation = handle_request("POST", "/?query=SELECT+1", b"DROP TABLE t", &config());
    assert_eq!(mutation.status, "400 Bad Request");

    let invalid_utf8 = handle_request("POST", "/?query=SELECT", b"\xff", &config());
    assert_eq!(invalid_utf8.status, "400 Bad Request");

    let almost_full = " ".repeat(rusthouse_server::MAX_QUERY_BYTES);
    let overflow = handle_request(
        "POST",
        "/?query=SELECT+1",
        almost_full.as_bytes(),
        &config(),
    );
    assert_eq!(overflow.status, "413 Payload Too Large");
}

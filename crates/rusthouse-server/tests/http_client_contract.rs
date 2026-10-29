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

#[test]
fn clickhouse_default_format_preserves_format_family_semantics() {
    let formats = [
        ("TSV", b"0\n1\n".as_slice()),
        ("TabSeparated", b"0\n1\n".as_slice()),
        ("TSVWithNames", b"number\n0\n1\n".as_slice()),
        ("TabSeparatedWithNames", b"number\n0\n1\n".as_slice()),
        ("CSV", b"0\r\n1\r\n".as_slice()),
        ("CSVWithNames", b"number\r\n0\r\n1\r\n".as_slice()),
        (
            "JSONStringsEachRow",
            b"{\"number\":\"0\"}\n{\"number\":\"1\"}\n".as_slice(),
        ),
        (
            "JSONCompactStringsEachRow",
            b"[\"0\"]\n[\"1\"]\n".as_slice(),
        ),
    ];
    for (name, expected) in formats {
        let target = format!("/?query=SELECT+number+FROM+numbers(2)&default_format={name}");
        let response = handle_request("GET", &target, b"", &config());
        assert_eq!(response.status, "200 OK", "{name}");
        assert_eq!(response.body, expected, "{name}");
    }
}

#[test]
fn default_format_is_bounded_and_explicit_format_wins() {
    for target in [
        "/?default_format=CSV&format=tsv",
        "/?format=tsv&default_format=CSV",
    ] {
        let response = handle_request("POST", target, b"SELECT 7", &config());
        assert_eq!(response.status, "200 OK", "{target}");
        assert_eq!(response.body, b"7\n7\n", "{target}");
    }
    for target in [
        "/?default_format=CSV&default_format=TSV",
        "/?default_format=does-not-exist",
        "/?default_format=JSONEachRow", // typed numbers need a distinct writer
        "/explain?default_format=TSV&query=SELECT+1",
    ] {
        assert_eq!(
            handle_request("GET", target, b"", &config()).status,
            "400 Bad Request",
            "{target}"
        );
    }
}

#[test]
fn clickhouse_sql_format_clause_overrides_http_default() {
    let get = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%282%29+FORMAT+CSV&default_format=TSVWithNames",
        b"",
        &config(),
    );
    assert_eq!(get.status, "200 OK");
    assert_eq!(get.body, b"0\r\n1\r\n");

    let post = handle_request(
        "POST",
        "/?default_format=CSVWithNames",
        b"SELECT 7 FORMAT TSV",
        &config(),
    );
    assert_eq!(post.status, "200 OK");
    assert_eq!(post.body, b"7\n");

    let semicolon = handle_request("POST", "/", b"SELECT 3 FORMAT TSV;", &config());
    assert_eq!(semicolon.status, "200 OK");
    assert_eq!(semicolon.body, b"3\n");

    let quoted = handle_request("POST", "/", b"SELECT 'FORMAT CSV' AS value", &config());
    assert_eq!(quoted.status, "200 OK");
    assert_eq!(quoted.body, b"value\nFORMAT CSV\n");
}

#[test]
fn clickhouse_sql_format_clause_rejects_bad_syntax() {
    for sql in [
        "SELECT 1 FORMAT JSONEachRow",
        "SELECT 1 FORMAT unknown",
        "SELECT 1 FORMAT CSV FORMAT TSV",
        "SELECT 1 FORMAT CSV; SELECT 2",
        "SELECT 1 FORMAT CSV /* unterminated",
    ] {
        assert_eq!(
            handle_request("POST", "/", sql.as_bytes(), &config()).status,
            "400 Bad Request",
            "{sql}"
        );
    }
    assert_eq!(
        handle_request("POST", "/explain", b"SELECT 1 FORMAT TSV", &config()).status,
        "400 Bad Request"
    );
}

#[test]
fn clickhouse_http_typed_query_parameters_bind_strings_and_integers() {
    let numeric = handle_request(
        "GET",
        "/?query=SELECT+%7Bx%3AInt32%7D+%2B+%7By%3AUInt16%7D&param_x=-3&param_y=5",
        b"",
        &config(),
    );
    assert_eq!(numeric.status, "200 OK");
    assert_eq!(numeric.body, b"2\n2\n");

    let filter = handle_request(
        "POST",
        "/?param_min=2",
        b"SELECT number FROM numbers(5) WHERE number >= {min:Int32}",
        &config(),
    );
    assert_eq!(filter.status, "200 OK");
    assert_eq!(filter.body, b"number\n2\n3\n4\n");

    let text = handle_request(
        "POST",
        "/?param_author=O%27Reilly",
        b"SELECT {author:String} AS author",
        &config(),
    );
    assert_eq!(text.status, "200 OK");
    assert_eq!(text.body, b"author\nO'Reilly\n");
}

#[test]
fn clickhouse_http_typed_parameters_reject_injection_and_bad_bindings() {
    for (target, sql) in [
        ("/?param_n=1%3BDROP+TABLE+t", "SELECT {n:Int32}"),
        ("/?param_n=999", "SELECT {n:UInt8}"),
        ("/?param_n=1", "SELECT {missing:Int32}"),
        ("/?param_n=1&param_n=2", "SELECT {n:Int32}"),
        ("/?param_n=1", "SELECT 1"),
        ("/?param_n=1", "SELECT '{n:Int32}'"),
        ("/?param_n=1", "DROP TABLE t"),
        ("/?param_n=1", "SELECT {n:Float64}"),
    ] {
        let result = handle_request("POST", target, sql.as_bytes(), &config());
        assert_eq!(result.status, "400 Bad Request", "{target} {sql}");
    }
}
#[test]
fn clickhouse_http_max_result_rows_limits_generated_results() {
    let default = handle_request(
        "POST",
        "/?max_result_rows=0",
        b"SELECT number FROM numbers(5)",
        &config(),
    );
    assert_eq!(default.status, "200 OK");
    assert_eq!(default.body, b"number\n0\n1\n2\n3\n4\n");

    let capped = handle_request(
        "POST",
        "/?max_result_rows=2",
        b"SELECT number FROM numbers(5)",
        &config(),
    );
    assert_eq!(capped.status, "200 OK");
    assert_eq!(capped.body, b"number\n0\n1\n");

    let explicit_limit = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%285%29+LIMIT+4&max_result_rows=1",
        b"",
        &config(),
    );
    assert_eq!(explicit_limit.status, "200 OK");
    assert_eq!(explicit_limit.body, b"number\n0\n");

    let no_relaxation = handle_request(
        "POST",
        "/?max_result_rows=500000",
        b"SELECT number FROM numbers(5)",
        &Config {
            max_rows: 3,
            ..config()
        },
    );
    assert_eq!(no_relaxation.status, "200 OK");
    assert_eq!(no_relaxation.body, b"number\n0\n1\n2\n");
}

#[test]
fn clickhouse_http_max_result_rows_validates_strict_numeric_values() {
    for target in [
        "/?max_result_rows=",
        "/?max_result_rows=-1",
        "/?max_result_rows=1.5",
        "/?max_result_rows=1%20",
        "/?max_result_rows=184467440737095516160",
        "/?max_result_rows=1&max_result_rows=2",
    ] {
        let response = handle_request("POST", target, b"SELECT 1", &config());
        assert_eq!(response.status, "400 Bad Request", "{target}");
    }
}
#[test]
fn clickhouse_explain_plan_sql_works_over_get_and_post() {
    let explicit = handle_request(
        "POST",
        "/",
        b"EXPLAIN PLAN SELECT number FROM numbers(100) WHERE number >= 10 LIMIT 5",
        &config(),
    );
    assert_eq!(explicit.status, "200 OK");
    assert_eq!(explicit.content_type, "text/plain; charset=utf-8");
    let plan = String::from_utf8(explicit.body).unwrap();
    assert!(plan.contains("Logical plan (no data read)"));
    assert!(plan.contains("Source: numbers(100)"));
    assert!(plan.contains("Filter:"));

    let implicit = handle_request(
        "GET",
        "/?query=explain+select+number+from+numbers%282%29",
        b"",
        &config(),
    );
    assert_eq!(implicit.status, "200 OK");
    assert!(String::from_utf8(implicit.body).unwrap().contains("Source: numbers(2)"));

    let capped = handle_request(
        "POST",
        "/?max_result_rows=2",
        b"EXPLAIN SELECT number FROM numbers(10)",
        &config(),
    );
    assert_eq!(capped.status, "200 OK");
    assert!(String::from_utf8(capped.body).unwrap().contains("Limit: 2"));
}

#[test]
fn clickhouse_explain_sql_rejects_mutations_and_unsupported_modes() {
    for sql in [
        "EXPLAIN",
        "EXPLAIN PLAN",
        "EXPLAIN AST SELECT 1",
        "EXPLAIN PIPELINE SELECT 1",
        "EXPLAIN INSERT INTO t VALUES (1)",
        "EXPLAIN PLAN DROP TABLE t",
        "EXPLAIN SELECT 1 FORMAT CSV",
    ] {
        let response = handle_request("POST", "/", sql.as_bytes(), &config());
        assert_eq!(response.status, "400 Bad Request", "{sql}");
    }
    let nested = handle_request("POST", "/explain", b"EXPLAIN SELECT 1", &config());
    assert_eq!(nested.status, "400 Bad Request");
}

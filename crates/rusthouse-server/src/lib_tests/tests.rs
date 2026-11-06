use super::*;
use std::fs;

fn config() -> Config {
    Config {
        part: None,
        database: None,
        max_rows: 3,
    }
}

#[test]
fn database_mode_reads_only_visible_matching_tables() {
    use rusthouse_storage::{Column, write_wide_part};
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT_DB: AtomicUsize = AtomicUsize::new(0);

    let database = std::env::temp_dir().join(format!(
        "rusthouse-server-db-{}-{}",
        std::process::id(),
        NEXT_DB.fetch_add(1, Ordering::Relaxed)
    ));
    let parts = database.join("events.parts");
    fs::create_dir_all(&parts).unwrap();
    let columns = vec![
        Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        },
        Column {
            name: "name".into(),
            data_type: "String".into(),
        },
    ];
    write_wide_part(
        &parts.join("all_0_0_0"),
        &columns,
        &[
            vec![Some("2".into()), Some("Grace".into())],
            vec![Some("1".into()), Some("Ada".into())],
        ],
    )
    .unwrap();
    fs::write(
        database.join("events.schema"),
        "rusthouse-schema-v1\nid\tUInt64\nname\tString\n",
    )
    .unwrap();
    let config = Config {
        part: None,
        database: Some(database.clone()),
        max_rows: 10,
    };
    let schema = handle_request("GET", "/schema?table=events", b"", &config);
    assert_eq!(schema.status, "200 OK");
    assert_eq!(schema.content_type, "application/json; charset=utf-8");
    assert_eq!(
        schema.body,
        b"{\"rows\":2,\"columns\":[{\"name\":\"id\",\"type\":\"UInt64\"},{\"name\":\"name\",\"type\":\"String\"}]}\n"
    );
    let result = handle_request(
        "POST",
        "/",
        b"SELECT id, name FROM events ORDER BY id",
        &config,
    );
    assert_eq!(result.status, "200 OK");
    assert_eq!(result.body, b"id\tname\n1\tAda\n2\tGrace\n");
    let count = handle_request("POST", "/", b"SELECT count(*) FROM events", &config);
    assert_eq!(count.body, b"count\n2\n");
    // SQL over HTTP must follow the same active generation as the CLI.
    let generation = parts.join(".rusthouse-versions").join("g1");
    fs::create_dir_all(&generation).unwrap();
    write_wide_part(
        &generation.join("all_1_1_0"),
        &columns,
        &[vec![Some("3".into()), Some("Lin".into())]],
    )
    .unwrap();
    fs::write(parts.join(".rusthouse-active"), "g1\n").unwrap();
    let current = handle_request(
        "POST",
        "/",
        b"SELECT id, name FROM events ORDER BY id",
        &config,
    );
    assert_eq!(current.status, "200 OK");
    assert_eq!(current.body, b"id\tname\n3\tLin\n");
    assert!(
        handle_request("GET", "/schema?table=events", b"", &config)
            .body
            .starts_with(b"{\"rows\":1,")
    );
    assert_eq!(
        handle_request("POST", "/", b"SELECT count(*) FROM events", &config).body,
        b"count\n1\n"
    );
    fs::write(parts.join(".rusthouse-active"), "g../old\n").unwrap();
    assert_eq!(
        handle_request("POST", "/", b"SELECT id FROM events", &config).status,
        "400 Bad Request"
    );
    fs::write(parts.join(".rusthouse-active"), "g1\n").unwrap();
    let mutation = handle_request("POST", "/", b"DROP TABLE events", &config);
    assert_eq!(mutation.status, "400 Bad Request");
    assert_eq!(
        handle_request("POST", "/", b"SELECT id FROM missing", &config).status,
        "400 Bad Request"
    );
    fs::write(
        database.join("events.schema"),
        "rusthouse-schema-v1\nid\tUInt8\nname\tString\n",
    )
    .unwrap();
    assert_eq!(
        handle_request("POST", "/", b"SELECT id FROM events", &config).status,
        "400 Bad Request"
    );
    fs::remove_file(database.join("events.schema")).unwrap();
    assert_eq!(
        handle_request("POST", "/", b"SELECT id FROM events", &config).status,
        "400 Bad Request"
    );
    fs::remove_dir_all(database).unwrap();
}

#[cfg(unix)]
#[test]
fn database_http_query_respects_cli_exclusive_lock() {
    use rusthouse_storage::{Column, acquire_database_lock, write_wide_part};
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT_DB: AtomicUsize = AtomicUsize::new(0);

    let database = std::env::temp_dir().join(format!(
        "rusthouse-server-lock-{}-{}",
        std::process::id(),
        NEXT_DB.fetch_add(1, Ordering::Relaxed)
    ));
    let parts = database.join("events.parts");
    fs::create_dir_all(&parts).unwrap();
    write_wide_part(
        &parts.join("all_0_0_0"),
        &[Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        }],
        &[vec![Some("5".into())]],
    )
    .unwrap();
    fs::write(
        database.join("events.schema"),
        "rusthouse-schema-v1\nid\tUInt64\n",
    )
    .unwrap();
    let config = Config {
        database: Some(database.clone()),
        part: None,
        max_rows: 10,
    };

    let writer = acquire_database_lock(&database, true).unwrap();
    let blocked = handle_request("POST", "/", b"SELECT id FROM events", &config);
    assert_eq!(blocked.status, "409 Conflict");
    assert_eq!(
        handle_request("GET", "/schema?table=events", b"", &config).status,
        "409 Conflict"
    );
    drop(writer);

    let allowed = handle_request("POST", "/", b"SELECT id FROM events", &config);
    assert_eq!(allowed.status, "200 OK");
    assert_eq!(allowed.body, b"id\n5\n");
    fs::remove_dir_all(database).unwrap();
}

#[test]
fn rejects_invalid_ping_methods_and_bodies() {
    assert_eq!(
        handle_request("GET", "/ping", b"payload", &config()).status,
        "400 Bad Request"
    );
    assert_eq!(
        handle_request("POST", "/ping", b"", &config()).status,
        "405 Method Not Allowed"
    );
    assert_eq!(
        handle_request("PUT", "/ping", b"", &config()).status,
        "405 Method Not Allowed"
    );
}

#[test]
fn accepts_post_sql_above_four_kib_but_rejects_oversized_bodies() {
    let statement = format!("SELECT number FROM numbers(1){}", " ".repeat(5000));
    let response = handle_request("POST", "/", statement.as_bytes(), &config());
    assert_eq!(response.status, "200 OK");
    assert_eq!(response.body, b"number\n0\n");
    let too_large = format!("SELECT 1{}", " ".repeat(MAX_QUERY_BYTES));
    assert_eq!(
        handle_request("POST", "/", too_large.as_bytes(), &config()).status,
        "413 Payload Too Large"
    );
}

#[test]
fn serves_health_and_bounded_select() {
    assert_eq!(
        handle_request("GET", "/ping", b"", &config()).body,
        b"Ok.\n"
    );
    let response = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%2810%29",
        b"",
        &config(),
    );
    assert_eq!(response.status, "200 OK");
    assert_eq!(response.body, b"number\n0\n1\n2\n");
}

#[test]
fn can_return_more_than_one_hundred_thousand_rows() {
    let config = Config {
        max_rows: 100_001,
        ..config()
    };
    let response = handle_request("POST", "/", b"SELECT number FROM numbers(100001)", &config);
    assert_eq!(response.status, "200 OK");
    assert_eq!(
        response.body.iter().filter(|&&byte| byte == b'\n').count(),
        100_002
    );
}

#[test]
fn advertises_capabilities_without_opening_source_files() {
    let default = handle_request("GET", "/capabilities", b"", &config());
    assert_eq!(default.status, "200 OK");
    assert_eq!(default.content_type, "application/json; charset=utf-8");
    let body = String::from_utf8(default.body).unwrap();
    assert!(body.contains("\"source\":\"generated\""));
    assert!(body.contains("\"read_only\":true"));
    assert!(body.contains("\"max_query_bytes\":65536"));
    assert!(body.contains("\"max_rows\":3"));
    assert!(!body.contains("/tmp/"));

    let part = Config {
        part: Some(PathBuf::from("/does/not/exist")),
        database: None,
        max_rows: 300_000,
    };
    let body = handle_request("GET", "/capabilities", b"", &part).body;
    let body = String::from_utf8(body).unwrap();
    assert!(body.contains("\"source\":\"part\""));
    assert!(body.contains("\"max_rows\":250000"));

    for (method, target, body, expected) in [
        (
            "POST",
            "/capabilities",
            b"".as_slice(),
            "405 Method Not Allowed",
        ),
        ("GET", "/capabilities", b"sql".as_slice(), "400 Bad Request"),
        (
            "GET",
            "/capabilities?query=SELECT+1",
            b"".as_slice(),
            "400 Bad Request",
        ),
    ] {
        assert_eq!(
            handle_request(method, target, body, &config()).status,
            expected
        );
    }
}

#[test]
fn rejects_mutations_invalid_encodings_and_conflicting_inputs() {
    for (method, target, body) in [
        ("POST", "/", &b"DROP TABLE events"[..]),
        ("GET", "/?query=%GG", &b""[..]),
        ("GET", "/?query=SELECT+1&query=SELECT+2", &b""[..]),
        ("POST", "/?query=SELECT+1", &b"SELECT 2"[..]),
        ("GET", "/unrecognized", &b""[..]),
    ] {
        assert_eq!(
            handle_request(method, target, body, &config()).status,
            "400 Bad Request"
        );
    }
}

#[test]
fn response_writer_accepts_above_eight_mib() {
    let mut writer = LimitedWriter::new();
    writer.write_all(&vec![b'x'; 8 * 1024 * 1024 + 1]).unwrap();
    assert_eq!(writer.data.len(), 8 * 1024 * 1024 + 1);
    assert!(!writer.exceeded);
}

#[test]
fn accepts_post_and_rejects_oversized_output() {
    let result = handle_request("POST", "/", b"SELECT 7", &config());
    assert_eq!(result.status, "200 OK");
    assert_eq!(result.body, b"7\n7\n");
    let mut writer = LimitedWriter {
        data: vec![0; MAX_RESPONSE_BYTES],
        exceeded: false,
    };
    assert!(writer.write_all(b"x").is_err());
    assert!(writer.exceeded);
    assert_eq!(writer.data.len(), MAX_RESPONSE_BYTES);
}

#[test]
fn exposes_a_read_only_json_schema_for_a_frozen_part() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../rusthouse-storage/tests/fixtures/all_1_1_0");
    let config = Config {
        part: Some(path),
        database: None,
        max_rows: 1,
    };
    let response = handle_request("GET", "/schema", b"", &config);
    assert_eq!(response.status, "200 OK");
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    assert_eq!(
        response.body,
        br#"{"rows":42,"columns":[{"name":"id","type":"UInt64"},{"name":"city","type":"LowCardinality(String)"}]}"#.iter().copied().chain(b"\n".iter().copied()).collect::<Vec<_>>()
    );

    for (method, path, body, status) in [
        ("POST", "/schema", &b""[..], "405 Method Not Allowed"),
        ("GET", "/schema", &b"x"[..], "400 Bad Request"),
        ("GET", "/schema?query=SELECT+1", &b""[..], "400 Bad Request"),
    ] {
        assert_eq!(handle_request(method, path, body, &config).status, status);
    }
}

#[test]
fn schema_discovery_requires_a_part_and_escapes_json() {
    let missing = handle_request("GET", "/schema", b"", &config());
    assert_eq!(missing.status, "400 Bad Request");
    for url in [
        "/schema?table=events",
        "/schema?table=",
        "/schema?table=events&table=other",
        "/schema?query=SELECT+1",
        "/schema?table=%FF",
    ] {
        assert_eq!(
            handle_request("GET", url, b"", &config()).status,
            "400 Bad Request",
            "{url}"
        );
    }
    let mut output = Vec::new();
    write_json_string(&mut output, "quote\"\\newline\n\u{07}é").unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\"quote\\\"\\\\newline\\n\\u0007é\""
    );
}

#[test]
fn explains_sql_without_scanning_data() {
    let get = handle_request(
        "GET",
        "/explain?query=SELECT+number+FROM+numbers%2810%29+WHERE+number%3E5",
        b"",
        &config(),
    );
    assert_eq!(get.status, "200 OK");
    assert_eq!(get.content_type, "text/plain; charset=utf-8");
    assert_eq!(
        String::from_utf8(get.body).unwrap(),
        "Logical plan (no data read):\nSource: numbers(10)\nFilter: 1 conditions across 1 OR branches\nProject: number\nLimit: 3\n"
    );

    // Planning a part query must not require --part or open data files.
    let planned = handle_request("POST", "/explain", b"SELECT id FROM part", &config());
    assert_eq!(planned.status, "200 OK");
    assert!(
        String::from_utf8(planned.body)
            .unwrap()
            .contains("Source: table part")
    );

    let executed = handle_request("POST", "/", b"SELECT id FROM part", &config());
    assert_eq!(executed.status, "400 Bad Request");
}

#[test]
fn explain_endpoint_rejects_writes_and_invalid_inputs() {
    for (method, path, body, status) in [
        (
            "GET",
            "/explain?query=DROP+TABLE+t",
            &b""[..],
            "400 Bad Request",
        ),
        ("GET", "/explain", &b"SELECT 1"[..], "400 Bad Request"),
        (
            "POST",
            "/explain?query=SELECT+1",
            &b"SELECT 2"[..],
            "400 Bad Request",
        ),
        (
            "PUT",
            "/explain",
            &b"SELECT 1"[..],
            "405 Method Not Allowed",
        ),
    ] {
        assert_eq!(handle_request(method, path, body, &config()).status, status);
    }
}

#[test]
fn safely_decodes_utf8_queries() {
    assert_eq!(decode_url("SELECT+1"), Ok("SELECT 1".into()));
    assert_eq!(decode_url("%FF"), Err("SQL query must be valid UTF-8"));
    assert_eq!(decode_url("%2"), Err("invalid percent encoding"));
}
#[test]
fn returns_bounded_ndjson_for_get_and_post() {
    let response = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%2810%29&format=jsonl",
        b"",
        &config(),
    );
    assert_eq!(response.status, "200 OK");
    assert_eq!(response.content_type, "application/x-ndjson; charset=utf-8");
    assert_eq!(
        response.body,
        b"{\"number\":\"0\"}\n{\"number\":\"1\"}\n{\"number\":\"2\"}\n"
    );

    let response = handle_request(
        "POST",
        "/?format=jsonl",
        b"SELECT count(*) FROM numbers(0)",
        &config(),
    );
    assert_eq!(response.status, "200 OK");
    assert_eq!(response.body, b"{\"count\":\"0\"}\n");

    let response = handle_request("POST", "/?format=tsv", b"SELECT 7", &config());
    assert_eq!(response.status, "200 OK");
    assert_eq!(response.body, b"7\n7\n");
}

#[test]
fn returns_json_arrays_for_get_and_post() {
    let response = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%2810%29&format=json",
        b"",
        &config(),
    );
    assert_eq!(response.status, "200 OK");
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    assert_eq!(
        response.body,
        b"[{\"number\":\"0\"},{\"number\":\"1\"},{\"number\":\"2\"}]\n"
    );

    let empty = handle_request(
        "POST",
        "/?format=json",
        b"SELECT number FROM numbers(0)",
        &config(),
    );
    assert_eq!(empty.status, "200 OK");
    assert_eq!(empty.body, b"[]\n");

    let count = handle_request(
        "POST",
        "/?format=json",
        b"SELECT count(*) FROM numbers(0)",
        &config(),
    );
    assert_eq!(count.status, "200 OK");
    assert_eq!(count.body, b"[{\"count\":\"0\"}]\n");

    let invalid = handle_request(
        "POST",
        "/?format=json",
        b"SELECT number, number FROM numbers(1)",
        &config(),
    );
    assert_eq!(invalid.status, "400 Bad Request");
}

#[test]
fn returns_csv_for_get_and_post_with_row_cap() {
    let get = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%2810%29&format=csv",
        b"",
        &config(),
    );
    assert_eq!(get.status, "200 OK");
    assert_eq!(get.content_type, "text/csv; charset=utf-8");
    assert_eq!(get.body, b"number\r\n0\r\n1\r\n2\r\n");

    let post = handle_request(
        "POST",
        "/?format=csv",
        b"SELECT count(*) FROM numbers(0)",
        &config(),
    );
    assert_eq!(post.status, "200 OK");
    assert_eq!(post.body, b"count\r\n0\r\n");

    let bad = handle_request("POST", "/?format=csv&format=tsv", b"SELECT 1", &config());
    assert_eq!(bad.status, "400 Bad Request");
}

#[test]
fn returns_csv_without_header_for_get_post_and_empty_results() {
    let get = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%2810%29&format=csv-no-header",
        b"",
        &config(),
    );
    assert_eq!(get.status, "200 OK");
    assert_eq!(get.content_type, "text/csv; charset=utf-8");
    assert_eq!(get.body, b"0\r\n1\r\n2\r\n");

    let post = handle_request(
        "POST",
        "/?format=csv-no-header",
        b"SELECT count(*) FROM numbers(0)",
        &config(),
    );
    assert_eq!(post.status, "200 OK");
    assert_eq!(post.body, b"0\r\n");

    let empty = handle_request(
        "POST",
        "/?format=csv-no-header",
        b"SELECT number FROM numbers(0)",
        &config(),
    );
    assert_eq!(empty.status, "200 OK");
    assert!(empty.body.is_empty());

    let explain = handle_request(
        "GET",
        "/explain?query=SELECT+1&format=csv-no-header",
        b"",
        &config(),
    );
    assert_eq!(explain.status, "400 Bad Request");
}

#[test]
fn rejects_unsupported_query_formats_and_duplicate_column_names() {
    for url in [
        "/?query=SELECT+1&format=xml",
        "/?query=SELECT+1&format=jsonl&format=jsonl",
        "/explain?query=SELECT+1&format=jsonl",
        "/?query=SELECT+number%2Cnumber+FROM+numbers%282%29&format=jsonl",
    ] {
        assert_eq!(
            handle_request("GET", url, b"", &config()).status,
            "400 Bad Request"
        );
    }
    let config = Config {
        part: None,
        database: None,
        max_rows: 0,
    };
    let empty = handle_request("POST", "/?format=jsonl", b"SELECT 1", &config);
    assert_eq!(empty.status, "200 OK");
    assert!(empty.body.is_empty());
}

#[test]
fn returns_tsv_without_column_headers_for_get_and_post() {
    let get = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%2810%29&format=tsv-no-header",
        b"",
        &config(),
    );
    assert_eq!(get.status, "200 OK");
    assert_eq!(get.content_type, "text/tab-separated-values; charset=utf-8");
    assert_eq!(get.body, b"0\n1\n2\n");

    let post = handle_request(
        "POST",
        "/?format=tsv-no-header",
        b"SELECT 7 AS value",
        &config(),
    );
    assert_eq!(post.status, "200 OK");
    assert_eq!(post.body, b"7\n");

    let cap = Config {
        part: None,
        database: None,
        max_rows: 0,
    };
    let empty = handle_request("POST", "/?format=tsv-no-header", b"SELECT 1", &cap);
    assert_eq!(empty.status, "200 OK");
    assert!(empty.body.is_empty());
}

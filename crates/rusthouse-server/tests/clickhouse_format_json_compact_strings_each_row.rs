use rusthouse_server::{Config, handle_request};

#[test]
fn clickhouse_http_json_compact_strings_each_row_emits_string_fields() {
    let config = Config { max_rows: 10, ..Default::default() };
    let response = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%282%29&format=JSONCompactStringsEachRow",
        b"",
        &config,
    );
    assert_eq!(response.status, "200 OK", "{:?}", response.body);
    assert_eq!(response.content_type, "application/x-ndjson; charset=utf-8");
    assert_eq!(response.body, b"[\"0\"]\n[\"1\"]\n");
}

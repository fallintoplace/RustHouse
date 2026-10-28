use rusthouse_server::{Config, handle_request};

#[test]
fn clickhouse_http_json_strings_each_row_emits_string_fields() {
    let config = Config { max_rows: 10, ..Default::default() };
    let response = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%282%29&format=JSONStringsEachRow",
        b"",
        &config,
    );
    assert_eq!(response.status, "200 OK", "{:?}", response.body);
    assert_eq!(response.content_type, "application/x-ndjson; charset=utf-8");
    assert_eq!(response.body, b"{\"number\":\"0\"}\n{\"number\":\"1\"}\n");
}

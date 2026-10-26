use rusthouse_server::{Config, handle_request};

fn config() -> Config {
    Config {
        max_rows: 1000,
        ..Default::default()
    }
}

#[test]
fn compact_arrays_support_post_and_get_without_renaming_columns() {
    let post = handle_request(
        "POST",
        "/?format=json-compact-each-row",
        b"SELECT 7 AS amount, 'hello' AS label, NULL AS missing",
        &config(),
    );
    assert_eq!(post.status, "200 OK", "{}", String::from_utf8_lossy(&post.body));
    assert_eq!(post.content_type, "application/x-ndjson; charset=utf-8");
    assert_eq!(post.body, b"[\"7\",\"hello\",null]\n");

    let get = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%283%29&format=json-compact-each-row",
        b"",
        &config(),
    );
    assert_eq!(get.status, "200 OK");
    assert_eq!(get.body, b"[\"0\"]\n[\"1\"]\n[\"2\"]\n");

    let empty = handle_request(
        "POST",
        "/?format=json-compact-each-row",
        b"SELECT number FROM numbers(0)",
        &config(),
    );
    assert_eq!(empty.status, "200 OK");
    assert!(empty.body.is_empty());
}

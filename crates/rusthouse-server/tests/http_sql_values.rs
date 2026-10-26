use rusthouse_server::{Config, handle_request};

fn config() -> Config {
    Config {
        max_rows: 2,
        ..Default::default()
    }
}

#[test]
fn sql_values_format_supports_get_and_bounded_rows() {
    let get = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%283%29&format=sql-values",
        b"",
        &config(),
    );
    assert_eq!(get.status, "200 OK");
    assert_eq!(get.content_type, "text/plain; charset=utf-8");
    assert_eq!(get.body, b"VALUES ('0'),\n('1');\n");
}

#[test]
fn sql_values_escapes_strings_and_nulls_and_omits_empty_results() {
    let post = handle_request(
        "POST",
        "/?format=sql-values",
        b"SELECT 7 AS id, 'O''Brien' AS name, NULL AS missing",
        &config(),
    );
    assert_eq!(post.status, "200 OK");
    assert_eq!(post.body, b"VALUES ('7', 'O''Brien', NULL);\n");

    let empty = handle_request(
        "POST",
        "/?format=sql-values",
        b"SELECT number FROM numbers(0)",
        &config(),
    );
    assert_eq!(empty.status, "200 OK");
    assert!(empty.body.is_empty());
}

use rusthouse_server::{Config, handle_request};

fn config() -> Config {
    Config { max_rows: 100, ..Config::default() }
}

#[test]
fn default_format_supports_clickhouse_url_setting() {
    let get = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%282%29&default_format=CSVWithNames",
        b"",
        &config(),
    );
    assert_eq!(get.status, "200 OK");
    assert_eq!(get.content_type, "text/csv; charset=utf-8");
    assert_eq!(get.body, b"number\r\n0\r\n1\r\n");

    let post = handle_request(
        "POST",
        "/?default_format=TabSeparated",
        b"SELECT number FROM numbers(2)",
        &config(),
    );
    assert_eq!(post.status, "200 OK");
    assert_eq!(post.body, b"0\n1\n");
}

#[test]
fn sql_format_overrides_default_but_not_explicit_url_format() {
    let sql_override = handle_request(
        "POST",
        "/?default_format=CSVWithNames",
        b"SELECT number FROM numbers(2) FORMAT TabSeparated",
        &config(),
    );
    assert_eq!(sql_override.status, "200 OK");
    assert_eq!(sql_override.body, b"0\n1\n");

    let url_override = handle_request(
        "POST",
        "/?default_format=CSVWithNames&format=json",
        b"SELECT 1",
        &config(),
    );
    assert_eq!(url_override.status, "200 OK");
    assert_eq!(url_override.body, b"[{\"1\":\"1\"}]\n");

    let conflict = handle_request(
        "POST",
        "/?format=csv",
        b"SELECT 1 FORMAT TabSeparated",
        &config(),
    );
    assert_eq!(conflict.status, "400 Bad Request");
}

#[test]
fn duplicate_or_unsupported_defaults_are_rejected() {
    for url in [
        "/?default_format=csv&default_format=json",
        "/?default_format=Unknown",
        "/explain?default_format=tsv",
    ] {
        let result = handle_request("POST", url, b"SELECT 1", &config());
        assert_eq!(result.status, "400 Bad Request", "{url}");
    }
}

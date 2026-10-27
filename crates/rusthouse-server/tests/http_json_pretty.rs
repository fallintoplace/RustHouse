use rusthouse_server::{Config, handle_request};

fn config() -> Config {
    Config {
        max_rows: 2,
        ..Default::default()
    }
}

#[test]
fn pretty_json_supports_get_post_empty_results_and_limits() {
    let get = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%283%29&format=json-pretty",
        b"",
        &config(),
    );
    assert_eq!(get.status, "200 OK");
    assert_eq!(get.content_type, "application/json; charset=utf-8");
    assert_eq!(
        get.body,
        b"[\n  {\n    \"number\": \"0\"\n  },\n  {\n    \"number\": \"1\"\n  }\n]\n"
    );

    let post = handle_request(
        "POST",
        "/?format=json-pretty",
        b"SELECT 'a,b' AS label, NULL AS missing",
        &config(),
    );
    assert_eq!(post.status, "200 OK");
    assert_eq!(
        post.body,
        b"[\n  {\n    \"label\": \"a,b\",\n    \"missing\": null\n  }\n]\n"
    );

    let empty = handle_request(
        "POST",
        "/?format=json-pretty",
        b"SELECT number FROM numbers(0)",
        &config(),
    );
    assert_eq!(empty.status, "200 OK");
    assert_eq!(empty.body, b"[]\n");
}

#[test]
fn pretty_json_rejects_duplicate_columns_and_unknown_formats() {
    let invalid = handle_request(
        "POST",
        "/?format=json-pretty",
        b"SELECT 1 AS id, 2 AS id",
        &config(),
    );
    assert_eq!(invalid.status, "400 Bad Request");

    let unsupported = handle_request(
        "POST",
        "/?format=json-prettyx",
        b"SELECT 1",
        &config(),
    );
    assert_eq!(unsupported.status, "400 Bad Request");
}

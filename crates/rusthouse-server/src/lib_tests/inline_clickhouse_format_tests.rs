use super::*;

#[test]
fn supports_inline_format_over_get_and_post() {
    let config = Config {
        part: None,
        database: None,
        max_rows: 100,
    };
    let get = handle_request(
        "GET",
        "/?query=SELECT+number+FROM+numbers%282%29+FORMAT+TabSeparated",
        b"",
        &config,
    );
    assert_eq!(get.status, "200 OK");
    assert_eq!(get.body, b"0\n1\n");
    let post = handle_request(
        "POST",
        "/",
        b"SELECT number FROM numbers(2) FORMAT JSONStringsEachRow",
        &config,
    );
    assert_eq!(post.status, "200 OK");
    assert_eq!(post.body, b"{\"number\":\"0\"}\n{\"number\":\"1\"}\n");
}

#[test]
fn rejects_double_format_and_unimplemented_typed_json() {
    let config = Config {
        part: None,
        database: None,
        max_rows: 100,
    };
    assert_eq!(
        handle_request("POST", "/?format=csv", b"SELECT 1 FORMAT CSV", &config).status,
        "400 Bad Request"
    );
    assert_eq!(
        handle_request("POST", "/", b"SELECT 1 FORMAT JSONEachRow", &config).status,
        "400 Bad Request"
    );
}

use super::*;

#[test]
fn accepts_standard_http_request_lines() {
    assert!(matches!(
        parse_request_line("GET /ping HTTP/1.1"),
        Ok(("GET", "/ping"))
    ));
    assert!(matches!(
        parse_request_line("POST / HTTP/1.0"),
        Ok(("POST", "/"))
    ));
}

#[test]
fn rejects_control_and_non_ascii_target_bytes() {
    for target in ["/\0", "/\t", "/\u{7f}", "/café"] {
        let line = format!("GET {target} HTTP/1.1");
        assert_eq!(
            parse_request_line(&line)
                .err()
                .map(|response| response.status),
            Some("400 Bad Request")
        );
    }
    assert!(parse_request_line("GET /?query=SELECT+1 HTTP/1.1").is_ok());
}

#[test]
fn rejects_tabs_repeated_spaces_and_trailing_spaces() {
    for line in [
        "GET\t/ping HTTP/1.1",
        "GET  /ping HTTP/1.1",
        "GET /ping  HTTP/1.1",
        "GET /ping HTTP/1.1 ",
        "GET\u{00a0}/ping HTTP/1.1",
    ] {
        assert_eq!(
            parse_request_line(line)
                .err()
                .map(|response| response.status),
            Some("400 Bad Request")
        );
    }
}

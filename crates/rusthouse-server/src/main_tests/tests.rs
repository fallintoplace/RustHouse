use super::*;

#[test]
fn accepts_http_token_header_names_and_optional_tabs() {
    assert_eq!(
        parse_header("Content-Length: 3").ok(),
        Some(("Content-Length", " 3"))
    );
    assert_eq!(
        parse_header("X_Request: hello\tworld").ok(),
        Some(("X_Request", " hello\tworld"))
    );
}

#[test]
fn rejects_invalid_header_names_and_embedded_controls() {
    for header in [
        "NoColon",
        ": value",
        "Content-Length : 3",
        "Bad Name: value",
        "X-Test: hello\u{0000}world",
        "X-Test: hello\u{001b}world",
    ] {
        assert_eq!(
            parse_header(header).unwrap_err().status,
            "400 Bad Request",
            "{header:?}"
        );
    }
}

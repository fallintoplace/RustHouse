use super::*;

#[test]
fn strips_only_http_optional_whitespace_from_control_headers() {
    assert_eq!(http_ows(" \t123\t "), "123");
    for invalid in ["\u{a0}123", "123\u{2003}", "\u{2002}123"] {
        assert_eq!(http_ows(invalid), invalid);
        assert!(!http_ows(invalid).bytes().all(|byte| byte.is_ascii_digit()));
    }
    let (_, value) = parse_header("Content-Length:\u{a0}123").unwrap();
    assert!(!http_ows(value).bytes().all(|byte| byte.is_ascii_digit()));
}

use super::*;

#[test]
fn rejects_unicode_whitespace_between_or_around_fields() {
    for text in [
        "node\u{00a0}zone 1\n",
        "node zone\u{2003}1\n",
        "\u{202f}node zone 1\n",
        "node zone 1\u{0085}\n",
    ] {
        assert!(parse_membership(text).is_err(), "{text:?}");
    }
    let members = parse_membership("n1\tzone\t1\r\nn2 zone 2\n").unwrap();
    assert_eq!(members.len(), 2);
    assert_eq!(
        format_membership(&members).unwrap(),
        "n1 zone 1\nn2 zone 2\n"
    );
}

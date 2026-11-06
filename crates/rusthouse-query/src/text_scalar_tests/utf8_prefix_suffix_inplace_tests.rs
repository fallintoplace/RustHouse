use crate::{ConstantValue, Projection, parse_select};

fn text(sql: &str) -> String {
    match parse_select(sql).unwrap().projection {
        Projection::Constants(columns) if columns.len() == 1 => {
            let ConstantValue::Text(value) = &columns[0].value else {
                panic!("expected text")
            };
            value.clone()
        }
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn left_utf8_counts_codepoints_not_bytes() {
    for (count, expected) in [(0, ""), (1, "é"), (2, "é🙂"), (3, "é🙂Z"), (9, "é🙂Z")]
    {
        let sql = format!("SELECT leftUTF8('é🙂Z', {count})");
        assert_eq!(text(&sql), expected);
    }
}

#[test]
fn right_utf8_scans_only_the_suffix_and_preserves_unicode() {
    for (count, expected) in [
        (0, ""),
        (1, "Z"),
        (2, "🙂Z"),
        (3, "é🙂Z"),
        (4, "aé🙂Z"),
        (9, "aé🙂Z"),
    ] {
        let sql = format!("SELECT rightUTF8('aé🙂Z', {count})");
        assert_eq!(text(&sql), expected);
    }
    let input = format!("{}🙂Z", "a".repeat(10_000));
    assert_eq!(text(&format!("SELECT rightUTF8('{input}', 2)")), "🙂Z");
}

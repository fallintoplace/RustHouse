use crate::{ConstantValue, Projection, parse_select};

#[test]
fn counts_distinct_bytes_including_utf8_and_duplicates() {
    for (sql, expected) in [
        ("SELECT stringBytesUniq('')", 0),
        ("SELECT stringBytesUniq('aaaa')", 1),
        ("SELECT stringBytesUniq('abcabc')", 3),
        ("SELECT stringBytesUniq('éé')", 2),
        ("SELECT stringBytesUniq('aé')", 3),
    ] {
        let parsed = parse_select(sql).unwrap();
        assert!(
            matches!(parsed.projection, Projection::Constants(values)
                if values.len() == 1
                    && values[0].value == ConstantValue::Integer(expected)),
            "{sql}",
        );
    }
    let parsed = parse_select("SELECT stringBytesUniq(NULL)").unwrap();
    assert!(matches!(parsed.projection, Projection::Constants(values)
        if values[0].value == ConstantValue::Null));
}

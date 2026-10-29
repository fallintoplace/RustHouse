use rusthouse_query::{CopyFormat, parse_copy_from, parse_json_each_row_fields};

#[test]
fn accepts_clickhouse_json_each_row_as_copy_format() {
    let copy = parse_copy_from(
        "COPY events FROM 'events.jsonl' WITH (FORMAT JSONEachRow)"
    ).unwrap().unwrap();
    assert_eq!(copy.format, CopyFormat::JsonEachRow);
    assert!(!copy.header);
    assert!(parse_copy_from(
        "COPY events FROM 'events.jsonl' WITH (FORMAT JSONEachRow, HEADER TRUE)"
    ).is_err());
}

#[test]
fn parses_named_scalar_values_and_json_escapes() {
    let fields = parse_json_each_row_fields(
        r#"{"id":18446744073709551615,"name":"café\uD83D\uDE80","unused":null,"enabled":true}"#
    ).unwrap();
    assert_eq!(fields, vec![
        ("id".into(), Some("18446744073709551615".into())),
        ("name".into(), Some("café🚀".into())),
        ("unused".into(), None),
        ("enabled".into(), Some("true".into())),
    ]);
}

#[test]
fn rejects_malformed_json_duplicate_names_and_non_objects() {
    for row in [
        r#"{"id":1,"id":2}"#,
        r#"{"id":1} trailing"#,
        r#"{"id":01}"#,
        r#"[1,2]"#,
        r#"{"empty": "bad\q"}"#,
        "{} {}",
    ] {
        assert!(parse_json_each_row_fields(row).is_err(), "{row}");
    }
}

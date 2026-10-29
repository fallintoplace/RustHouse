use rusthouse_query::{InsertFormatHeader, parse_insert, parse_insert_format_header};

#[test]
fn parses_clickhouse_streaming_json_insert_header() {
    assert_eq!(
        parse_insert_format_header("INSERT INTO events (id, name) FORMAT JSONEachRow;")
            .unwrap(),
        Some(InsertFormatHeader {
            table: "events".into(),
            columns: Some(vec!["id".into(), "name".into()]),
        })
    );
    assert_eq!(
        parse_insert_format_header("insert into events format jsoneachrow").unwrap(),
        Some(InsertFormatHeader { table: "events".into(), columns: None })
    );
    assert!(parse_insert_format_header("INSERT INTO events VALUES (1)").unwrap().is_none());
    assert!(parse_insert("INSERT INTO events VALUES (1)").unwrap().is_some());
}

#[test]
fn rejects_bad_formats_duplicate_columns_and_trailing_tokens() {
    for sql in [
        "INSERT INTO events FORMAT CSV",
        "INSERT INTO events (id, id) FORMAT JSONEachRow",
        "INSERT INTO events FORMAT JSONEachRow; DROP TABLE events",
        "INSERT INTO events FORMAT JSONEachRow extra",
    ] {
        assert!(parse_insert_format_header(sql).is_err(), "{sql}");
    }
}

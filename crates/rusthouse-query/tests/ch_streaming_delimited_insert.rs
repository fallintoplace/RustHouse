use rusthouse_query::{InsertStreamFormat, parse_insert_format_header};

#[test]
fn supports_clickhouse_streaming_delimited_formats() {
    for (name, expected) in [
        ("CSV", InsertStreamFormat::Csv),
        ("CSVWithNames", InsertStreamFormat::CsvWithNames),
        ("TabSeparated", InsertStreamFormat::TabSeparated),
        ("TabSeparatedWithNames", InsertStreamFormat::TabSeparatedWithNames),
        ("JSONEachRow", InsertStreamFormat::JsonEachRow),
    ] {
        let parsed = parse_insert_format_header(
            &format!("INSERT INTO events FORMAT {name}")
        ).unwrap().unwrap();
        assert_eq!(parsed.format, expected, "{name}");
        assert_eq!(parsed.table, "events");
    }
}

#[test]
fn rejects_unimplemented_and_extra_format_settings() {
    for sql in [
        "INSERT INTO events FORMAT JSON",
        "INSERT INTO events FORMAT Pretty",
        "INSERT INTO events FORMAT CSVWithNames SETTINGS foo=1",
        "INSERT INTO events FORMAT CSV FORMAT CSV",
    ] {
        assert!(parse_insert_format_header(sql).is_err(), "{sql}");
    }
}

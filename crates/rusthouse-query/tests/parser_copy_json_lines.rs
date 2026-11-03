use rusthouse_query::{CopyFormat, parse_copy_from};

#[test]
fn json_lines_format_names_map_to_json_each_row() {
    for format in ["JSONL", "NDJSON", "JSONEACHROW"] {
        let sql = format!("COPY events FROM 'events.jsonl' WITH (FORMAT {format})");
        let copy = parse_copy_from(&sql).unwrap().unwrap();
        assert_eq!(copy.format, CopyFormat::JsonEachRow);
        assert!(!copy.header);
    }
}

#[test]
fn json_lines_formats_reject_csv_header() {
    for format in ["JSONL", "NDJSON"] {
        let sql = format!("COPY events FROM 'events.jsonl' WITH (FORMAT {format}, HEADER TRUE)");
        assert!(parse_copy_from(&sql).is_err(), "{sql}");
    }
}

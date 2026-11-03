use rusthouse_query::parse_copy_from;

#[test]
fn copy_header_accepts_only_zero_and_one() {
    let yes = parse_copy_from("COPY t FROM 'data.csv' WITH (FORMAT CSV, HEADER 1)")
        .unwrap()
        .unwrap();
    let no = parse_copy_from("COPY t FROM 'data.csv' WITH (FORMAT CSV, HEADER 0)")
        .unwrap()
        .unwrap();
    assert!(yes.header);
    assert!(!no.header);
    for invalid in ["2", "-1", "1.0"] {
        let sql = format!("COPY t FROM 'data.csv' WITH (FORMAT CSV, HEADER {invalid})");
        assert!(parse_copy_from(&sql).is_err(), "{sql}");
    }
}

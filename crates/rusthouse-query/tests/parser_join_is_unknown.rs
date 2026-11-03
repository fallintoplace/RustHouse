use rusthouse_query::parse_join;

#[test]
fn join_is_unknown_maps_to_null_check() {
    let base = "SELECT a.id FROM a JOIN b ON a.id = b.id WHERE a.flag IS";
    let unknown = parse_join(&format!("{base} UNKNOWN")).unwrap().unwrap();
    let null = parse_join(&format!("{base} NULL")).unwrap().unwrap();
    assert_eq!(unknown.where_null, null.where_null);

    let not_unknown = parse_join(&format!("{base} NOT UNKNOWN")).unwrap().unwrap();
    let not_null = parse_join(&format!("{base} NOT NULL")).unwrap().unwrap();
    assert_eq!(not_unknown.where_null, not_null.where_null);

    assert!(parse_join(&format!("{base} KNOWN")).is_err());
}

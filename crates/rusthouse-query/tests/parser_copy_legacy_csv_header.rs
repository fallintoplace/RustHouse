use rusthouse_query::parse_copy_from;

#[test]
fn legacy_copy_csv_options_match_parenthesized_options() {
    let base = "COPY events FROM 'events.csv'";
    let legacy_header = parse_copy_from(&format!("{base} WITH CSV HEADER")).unwrap();
    let modern_header = parse_copy_from(&format!("{base} WITH (FORMAT CSV, HEADER TRUE)")).unwrap();
    assert_eq!(legacy_header, modern_header);

    let legacy = parse_copy_from(&format!("{base} WITH CSV")).unwrap();
    let modern = parse_copy_from(&format!("{base} WITH (FORMAT CSV)")).unwrap();
    assert_eq!(legacy, modern);

    assert!(parse_copy_from(&format!("{base} WITH CSV HEADER DELIMITER ','")).is_err());
}

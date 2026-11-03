use rusthouse_query::parse_copy_from;

#[test]
fn copy_header_yes_no_matches_true_false() {
    let canonical_yes = parse_copy_from("COPY t FROM 'data.csv' WITH (FORMAT CSV, HEADER TRUE)").unwrap();
    let canonical_no = parse_copy_from("COPY t FROM 'data.csv' WITH (FORMAT CSV, HEADER FALSE)").unwrap();
    assert_eq!(
        parse_copy_from("COPY t FROM 'data.csv' WITH (FORMAT CSV, HEADER YES)").unwrap(),
        canonical_yes,
    );
    assert_eq!(
        parse_copy_from("COPY t FROM 'data.csv' WITH (FORMAT CSV, HEADER no)").unwrap(),
        canonical_no,
    );
    assert!(parse_copy_from("COPY t FROM 'data.csv' WITH (FORMAT CSV, HEADER MAYBE)").is_err());
}

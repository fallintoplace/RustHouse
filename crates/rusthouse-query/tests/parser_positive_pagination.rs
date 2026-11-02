use rusthouse_query::parse_select;

#[test]
fn unary_plus_pagination_matches_unsigned_counts() {
    for (plus, plain) in [
        (
            "SELECT number FROM numbers(8) LIMIT +3 OFFSET +2",
            "SELECT number FROM numbers(8) LIMIT 3 OFFSET 2",
        ),
        (
            "SELECT number FROM numbers(8) LIMIT +2, +3",
            "SELECT number FROM numbers(8) LIMIT 2, 3",
        ),
        (
            "SELECT number FROM numbers(8) OFFSET +2 ROWS FETCH NEXT +3 ROWS ONLY",
            "SELECT number FROM numbers(8) OFFSET 2 ROWS FETCH NEXT 3 ROWS ONLY",
        ),
    ] {
        assert_eq!(parse_select(plus).unwrap(), parse_select(plain).unwrap());
    }
    for invalid in [
        "SELECT number FROM numbers(8) LIMIT ++2",
        "SELECT number FROM numbers(8) OFFSET +",
        "SELECT number FROM numbers(8) FETCH FIRST +-2 ROWS ONLY",
    ] {
        assert!(parse_select(invalid).is_err(), "{invalid}");
    }
}

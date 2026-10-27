use rusthouse_query::{Projection, parse_select};

#[test]
fn width_bucket_handles_boundaries_and_signed_ranges() {
    for (sql, expected) in [
        ("SELECT WIDTH_BUCKET(-1, 0, 10, 4)", 0),
        ("SELECT WIDTH_BUCKET(0, 0, 10, 4)", 1),
        ("SELECT WIDTH_BUCKET(2, 0, 10, 4)", 1),
        ("SELECT width_bucket(3, 0, 10, 4)", 2),
        ("SELECT WIDTH_BUCKET(5, 0, 10, 4)", 3),
        ("SELECT WIDTH_BUCKET(9, 0, 10, 4)", 4),
        ("SELECT WIDTH_BUCKET(10, 0, 10, 4)", 5),
        ("SELECT WIDTH_BUCKET(11, 0, 10, 4)", 5),
        ("SELECT WIDTH_BUCKET(0, -10, 10, 2)", 2),
        (
            "SELECT WIDTH_BUCKET(0, -9223372036854775808, 9223372036854775807, 10)",
            6,
        ),
        ("SELECT WIDTH_BUCKET(1 + 1, 0, 4, 2) * 2", 4),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn width_bucket_rejects_reversed_ranges_invalid_counts_and_bad_arity() {
    for sql in [
        "SELECT WIDTH_BUCKET(5, 10, 0, 4)",
        "SELECT WIDTH_BUCKET(5, 0, 0, 4)",
        "SELECT WIDTH_BUCKET(5, 0, 10, 0)",
        "SELECT WIDTH_BUCKET(5, 0, 10, -1)",
        "SELECT WIDTH_BUCKET(5, 0, 10, 1000001)",
        "SELECT WIDTH_BUCKET()",
        "SELECT WIDTH_BUCKET(1, 2, 3)",
        "SELECT WIDTH_BUCKET(1, 2, 3, 4, 5)",
        "SELECT WIDTH_BUCKET(1, 2, 3,)",
        "SELECT WIDTH_BUCKET(1, 2, 3, 4",
        "SELECT WIDTH_BUCKET('x', 0, 10, 4)",
        "SELECT WIDTH_BUCKET(1, 0, 2, 4) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

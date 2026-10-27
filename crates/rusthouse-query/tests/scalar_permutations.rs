use rusthouse_query::{Projection, parse_select};

#[test]
fn scalar_permutations_evaluates_bounded_constants() {
    for (sql, expected) in [
        ("SELECT PERMUTATIONS(5, 2)", 20),
        ("SELECT permutations(5, 0)", 1),
        ("SELECT PERMUTATIONS(0, 0)", 1),
        ("SELECT PERMUTATIONS(6, 3)", 120),
        ("SELECT PERMUTATIONS(20, 20)", 2432902008176640000),
        ("SELECT PERMUTATIONS(20, 1)", 20),
        ("SELECT PERMUTATIONS(2 + 3, 2)", 20),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(expected),
            "{sql}"
        );
    }
}

#[test]
fn scalar_permutations_rejects_invalid_arguments() {
    for sql in [
        "SELECT PERMUTATIONS(-1, 0)",
        "SELECT PERMUTATIONS(5, -1)",
        "SELECT PERMUTATIONS(5, 6)",
        "SELECT PERMUTATIONS(21, 1)",
        "SELECT PERMUTATIONS(1)",
        "SELECT PERMUTATIONS(1, 0, 0)",
        "SELECT PERMUTATIONS('x', 1)",
        "SELECT PERMUTATIONS(1, 0) FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

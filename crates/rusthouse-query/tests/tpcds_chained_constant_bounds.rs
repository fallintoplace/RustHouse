use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn between_folds_chained_addition_and_subtraction() {
    assert_eq!(
        run("SELECT number FROM numbers(9) WHERE number BETWEEN 3 AND 3 + 4 - 2 ORDER BY number"),
        "number\n3\n4\n5\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(9) WHERE number BETWEEN 2 + 1 AND 2 + 1 + 2 ORDER BY number"
        ),
        "number\n3\n4\n5\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(9) WHERE number NOT BETWEEN 2 + 1 AND 2 + 4 - 1 ORDER BY number"
        ),
        "number\n0\n1\n2\n6\n7\n8\n"
    );
}

#[test]
fn malformed_and_overflowed_arithmetic_is_rejected() {
    for sql in [
        "SELECT number FROM numbers(9) WHERE number BETWEEN 1 AND 2 +",
        "SELECT number FROM numbers(9) WHERE number BETWEEN 1 AND 2 + 3 -",
        "SELECT number FROM numbers(9) WHERE number BETWEEN 1 AND 170141183460469231731687303715884105727 + 1",
        "SELECT number FROM numbers(9) WHERE number BETWEEN 1 AND 2 + '3'",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

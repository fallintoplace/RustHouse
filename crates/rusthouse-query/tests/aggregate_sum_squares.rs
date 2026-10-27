use rusthouse_query::{execute, parse_select};

fn query(sql: &str) -> Result<String, String> {
    let select = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&select, None, &mut output).map_err(|error| error.to_string())?;
    Ok(String::from_utf8(output).unwrap())
}

#[test]
fn sum_squares_supports_filters_and_groups() {
    assert_eq!(
        query("SELECT sumSquares(number) FROM numbers(4)").unwrap(),
        "sumSquares(number)\n14\n"
    );
    assert_eq!(
        query("SELECT sumSquares(number) FROM numbers(4) WHERE number >= 2").unwrap(),
        "sumSquares(number)\n13\n"
    );
    assert_eq!(
        query("SELECT number, sumSquares(number) FROM numbers(3) GROUP BY number").unwrap(),
        "number\tsumSquares(number)\n0\t0\n1\t1\n2\t4\n"
    );
}

#[test]
fn sum_squares_checks_integer_overflow_and_empty_results() {
    assert_eq!(
        query("SELECT sumSquares(number) FROM numbers(0)").unwrap(),
        "sumSquares(number)\nNULL\n"
    );
    assert!(
        query("SELECT sumSquares(number) FROM numbers(18446744073709551614, 2)")
            .unwrap_err()
            .contains("overflow")
    );
}

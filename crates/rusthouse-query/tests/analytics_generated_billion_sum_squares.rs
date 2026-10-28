//! Exact checked sum of squared metrics over billion-row analytical inputs.
use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> Result<String, String> {
    let mut out = Vec::new();
    execute(&parse_select(sql).map_err(|e| e.to_string())?, None, &mut out)
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8(out).unwrap())
}

#[test]
fn billion_row_sum_squares_is_exact_without_a_billion_row_scan() {
    assert_eq!(
        run("SELECT sumSquares(number), count(*) FROM numbers(1000000000)").unwrap(),
        "sumSquares(number)\tcount(*)\n333333332833333333500000000\t1000000000\n"
    );
}

#[test]
fn nonzero_and_filtered_strides_use_exact_polynomial() {
    assert_eq!(
        run("SELECT sumSquares(number), sum(number) FROM generate_series(2, 6, 2)").unwrap(),
        "sumSquares(number)\tsum(number)\n56\t12\n"
    );
    assert_eq!(
        run("SELECT sumSquares(number), count(*) FROM numbers(1000000000) WHERE number < 3").unwrap(),
        "sumSquares(number)\tcount(*)\n5\t3\n"
    );
}

#[test]
fn excessive_sum_squares_reports_overflow_instead_of_wrapping() {
    let error = run("SELECT sumSquares(number), count(*) FROM generate_series(18446744073709551600, 18446744073709551615)").unwrap_err();
    assert!(error.contains("SUMSQUARES overflow"), "{error}");
}

#[test]
fn empty_sum_squares_remains_null() {
    assert_eq!(
        run("SELECT sumSquares(number), count(*) FROM numbers(0)").unwrap(),
        "sumSquares(number)\tcount(*)\nNULL\t0\n"
    );
}

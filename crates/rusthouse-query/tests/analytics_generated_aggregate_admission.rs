//! Billion-row virtual inputs only admit proven constant-time multi-aggregates.
use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    Ok(String::from_utf8(output).expect("TSV must be UTF-8"))
}

#[test]
fn billion_row_multi_metrics_use_formula_without_scanning() {
    assert_eq!(
        run("SELECT count(*), sum(number), max(number) FROM numbers(1000000000)").unwrap(),
        "count(*)\tsum(number)\tmax(number)\n1000000000\t499999999500000000\t999999999\n"
    );
}

#[test]
fn unsupported_billion_row_metrics_are_rejected_before_scan() {
    let error = run("SELECT sumSquares(number), count(*) FROM numbers(1000000000)").unwrap_err();
    assert!(error.contains("virtual inputs over 10000000 rows"), "{error}");
}

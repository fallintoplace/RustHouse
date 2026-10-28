//! Range-filtered billion-row generated metrics must never scan all rows.
use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> Result<String, String> {
    let query = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&query, None, &mut output).map_err(|error| error.to_string())?;
    Ok(String::from_utf8(output).unwrap())
}

#[test]
fn billion_row_range_metrics_use_exact_filtered_interval() {
    assert_eq!(
        run("SELECT count(*), sum(number), min(number), max(number), median(number) FROM numbers(1000000000) WHERE number BETWEEN 100000000 AND 899999999").unwrap(),
        "count(*)\tsum(number)\tmin(number)\tmax(number)\tmedian(number)\n800000000\t399999999600000000\t100000000\t899999999\t499999999.5\n"
    );
}

#[test]
fn generated_range_filters_handle_empty_and_strided_series() {
    assert_eq!(
        run("SELECT count(*), sum(number) FROM numbers(1000000000) WHERE number > 1000000000").unwrap(),
        "count(*)\tsum(number)\n0\tNULL\n"
    );
    assert_eq!(
        run("SELECT sum(number), count(*), range(number) FROM generate_series(10, 18, 2) WHERE number >= 14").unwrap(),
        "sum(number)\tcount(*)\trange(number)\n48\t3\t4\n"
    );
}

#[test]
fn sparse_billion_row_filters_remain_bounded() {
    let error = run("SELECT sum(number), count(*) FROM numbers(1000000000) WHERE number % 2 = 0").unwrap_err();
    assert!(error.contains("virtual inputs over 10000000 rows"), "{error}");
}

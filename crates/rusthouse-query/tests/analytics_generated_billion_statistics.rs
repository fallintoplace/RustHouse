//! Formula-backed statistics over very large generated analytical inputs.
use rusthouse_query::{execute, parse_select};

fn values(sql: &str) -> Vec<String> {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    let rendered = String::from_utf8(output).unwrap();
    rendered.lines().nth(1).unwrap().split('\t').map(str::to_owned).collect()
}

#[test]
fn billion_row_median_range_any_and_countif_do_not_scan() {
    assert_eq!(
        values("SELECT median(number), range(number), any(number), anyLast(number), countIf(number) FROM numbers(1000000000)"),
        ["499999999.5", "999999999", "0", "999999999", "999999999"]
    );
}

#[test]
fn exact_progression_statistics_follow_sample_and_population_definitions() {
    let cells = values(
        "SELECT varPop(number), varSamp(number), stddevPop(number), stddevSamp(number), rms(number) FROM generate_series(10, 18, 2)"
    );
    let expected = [8.0_f64, 10.0, 8.0_f64.sqrt(), 10.0_f64.sqrt(), 204.0_f64.sqrt()];
    for (actual, expected) in cells.iter().zip(expected) {
        let actual: f64 = actual.parse().unwrap();
        assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
    }
}

#[test]
fn zero_and_singleton_rows_match_null_aggregate_semantics() {
    assert_eq!(
        values("SELECT median(number), varPop(number), varSamp(number), rms(number), countIf(number) FROM numbers(0)"),
        ["NULL", "NULL", "NULL", "NULL", "0"]
    );
    assert_eq!(
        values("SELECT median(number), varPop(number), varSamp(number), countIf(number) FROM generate_series(0, 0)"),
        ["0", "0", "NULL", "0"]
    );
}

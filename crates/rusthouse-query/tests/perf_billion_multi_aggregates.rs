use rusthouse_query::{execute, parse_select};

#[test]
fn billion_row_multi_metric_series_runs_via_exact_formulas() {
    let mut output = Vec::new();
    execute(
        &parse_select("SELECT sum(number), count(*), min(number), max(number) FROM numbers(1000000000)").unwrap(),
        None,
        &mut output,
    ).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "sum(number)\tcount(*)\tmin(number)\tmax(number)\n499999999500000000\t1000000000\t0\t999999999\n",
    );
}

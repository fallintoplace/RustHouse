use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}
#[test]
fn computes_monotonic_filtered_multimetric_series_without_scan() {
    assert_eq!(
        run("SELECT sum(number), count(*), min(number), max(number) FROM numbers(5000000) WHERE number >= 4999998"),
        "sum(number)\tcount(*)\tmin(number)\tmax(number)\n9999997\t2\t4999998\t4999999\n"
    );
    assert_eq!(
        run("SELECT sum(number), count(*) FROM numbers(5) WHERE number >= 20"),
        "sum(number)\tcount(*)\nNULL\t0\n"
    );
}

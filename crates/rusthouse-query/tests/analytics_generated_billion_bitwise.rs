//! Bitwise range aggregation over billions of consecutive source keys.
use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> Result<String, String> {
    let mut out = Vec::new();
    execute(&parse_select(sql).map_err(|e|e.to_string())?, None, &mut out)
        .map_err(|e|e.to_string())?;
    Ok(String::from_utf8(out).unwrap())
}

#[test]
fn billion_row_bitwise_aggregates_use_exact_prefix_formulas() {
    assert_eq!(
        run("SELECT groupBitXor(number), groupBitAnd(number), groupBitOr(number) FROM numbers(1000000000)").unwrap(),
        "groupBitXor(number)\tgroupBitAnd(number)\tgroupBitOr(number)\n0\t0\t1073741823\n"
    );
}
#[test]
fn bitwise_extrema_and_singletons_preserve_exact_values() {
    assert_eq!(
        run("SELECT groupBitAnd(number), groupBitOr(number), groupBitXor(number) FROM generate_series(8, 15)").unwrap(),
        "groupBitAnd(number)\tgroupBitOr(number)\tgroupBitXor(number)\n8\t15\t0\n"
    );
    assert_eq!(
        run("SELECT groupBitAnd(number), groupBitOr(number) FROM numbers(0)").unwrap(),
        "groupBitAnd(number)\tgroupBitOr(number)\nNULL\tNULL\n"
    );
}
#[test]
fn strided_series_stays_bounded_until_implemented_exactly() {
    let error = run("SELECT groupBitXor(number), count(*) FROM generate_series(0, 20000000, 2)").unwrap_err();
    assert!(error.contains("virtual inputs over 10000000 rows"), "{error}");
    assert_eq!(
        run("SELECT groupBitXor(number), groupBitOr(number) FROM generate_series(0, 6, 2)").unwrap(),
        "groupBitXor(number)\tgroupBitOr(number)\n0\t6\n"
    );
}

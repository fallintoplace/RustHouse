//! ClickHouse-style analytical aggregate families must work in one shared scan.
use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut result = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut result).unwrap();
    String::from_utf8(result).unwrap()
}

#[test]
fn mixed_numeric_aggregates_use_one_generated_scan() {
    assert_eq!(
        run("SELECT sumSquares(number), product(number), countIf(number), count(*) FROM numbers(4)"),
        "sumSquares(number)\tproduct(number)\tcountIf(number)\tcount(*)\n14\t0\t3\t4\n"
    );
    assert_eq!(
        run("SELECT any(number), anyLast(number), min(number), max(number), range(number) FROM numbers(5) WHERE number > 1"),
        "any(number)\tanyLast(number)\tmin(number)\tmax(number)\trange(number)\n2\t4\t2\t4\t2\n"
    );
    assert_eq!(
        run("SELECT groupBitXor(number), groupBitAnd(number), groupBitOr(number), count(*) FROM numbers(4) WHERE number > 0"),
        "groupBitXor(number)\tgroupBitAnd(number)\tgroupBitOr(number)\tcount(*)\n0\t0\t3\t3\n"
    );
}

#[test]
fn statistics_share_one_pass_and_handle_empty_input() {
    let output = run("SELECT varPop(number), varSamp(number), stddevPop(number), stddevSamp(number), median(number), rms(number) FROM numbers(4)");
    let cells = output.lines().nth(1).unwrap().split('\t')
        .map(|value| value.parse::<f64>().unwrap()).collect::<Vec<_>>();
    assert_eq!(cells.len(), 6);
    for (actual, expected) in cells.iter().zip([
        1.25_f64, 5.0 / 3.0, 1.25_f64.sqrt(),
        (5.0_f64 / 3.0).sqrt(), 1.5, (3.5_f64).sqrt(),
    ]) {
        assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
    }
    assert_eq!(
        run("SELECT countIf(number), median(number), varPop(number) FROM numbers(0)"),
        "countIf(number)\tmedian(number)\tvarPop(number)\n0\tNULL\tNULL\n"
    );
}

#[test]
fn extended_families_are_available_with_existing_aggregates() {
    for name in [
        "countIf", "product", "sumSquares", "range", "rms", "varPop",
        "varSamp", "stddevPop", "stddevSamp", "median", "any", "anyLast",
        "groupBitAnd", "groupBitOr", "groupBitXor",
    ] {
        let sql = format!("SELECT {name}(number), count(*) FROM numbers(2)");
        let result = run(&sql);
        assert_eq!(result.lines().count(), 2, "{sql}");
    }
}

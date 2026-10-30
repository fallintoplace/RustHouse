use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn ordered_generated_single_aggregate_skips_billion_row_scan() {
    assert_eq!(
        run("SELECT number, sum(number) FROM numbers(1000000000) GROUP BY number ORDER BY number DESC LIMIT 2 OFFSET 1"),
        "number\tsum(number)\n999999998\t999999998\n999999997\t999999997\n"
    );
    assert_eq!(
        run("SELECT number, max(number) FROM numbers(1000000000) WHERE number >= 999999995 GROUP BY number ORDER BY number DESC LIMIT 2 OFFSET 1"),
        "number\tmax(number)\n999999998\t999999998\n999999997\t999999997\n"
    );
}

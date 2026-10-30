use rusthouse_query::{execute, parse_select};

#[test]
fn streams_unbounded_descending_multi_metric_groups() {
    let query = parse_select(
        "SELECT number, sum(number), count(number) FROM numbers(5000000) WHERE number >= 4999997 GROUP BY number ORDER BY number DESC OFFSET 1",
    ).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "number\tsum(number)\tcount(number)\n4999998\t4999998\t1\n4999997\t4999997\t1\n",
    );
}

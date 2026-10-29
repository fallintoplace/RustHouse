use rusthouse_query::{execute, parse_select};

#[test]
fn filtered_multiple_metrics_skip_irrelevant_generated_rows() {
    let query = parse_select(
        "SELECT number, sum(number), count(number) FROM numbers(5000000) WHERE number >= 4999997 GROUP BY number OFFSET 1",
    ).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "number\tsum(number)\tcount(number)\n4999998\t4999998\t1\n4999999\t4999999\t1\n",
    );
}

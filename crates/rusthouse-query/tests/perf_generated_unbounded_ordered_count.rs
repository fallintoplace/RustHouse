use rusthouse_query::{execute, parse_select};

#[test]
fn streams_descending_generated_count_groups_without_materialization() {
    let query = parse_select(
        "SELECT number, count(*) FROM numbers(5000000) WHERE number >= 4999997 GROUP BY number ORDER BY number DESC OFFSET 1",
    ).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "number\tcount\n4999998\t1\n4999997\t1\n",
    );
}

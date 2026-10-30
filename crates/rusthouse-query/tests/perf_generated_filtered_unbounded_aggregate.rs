use rusthouse_query::{execute, parse_select};

#[test]
fn filtered_group_aggregate_jumps_to_matched_range() {
    let query = parse_select(
        "SELECT number, max(number) FROM numbers(5000000) WHERE number >= 4999997 GROUP BY number OFFSET 1",
    ).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "number\tmax(number)\n4999998\t4999998\n4999999\t4999999\n",
    );
}

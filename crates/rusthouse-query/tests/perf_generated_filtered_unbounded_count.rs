use rusthouse_query::{execute, parse_select};

#[test]
fn filtered_group_count_skips_millions_of_irrelevant_keys() {
    let query = parse_select(
        "SELECT number, count(*) FROM numbers(5000000) WHERE number >= 4999997 GROUP BY number OFFSET 1",
    ).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "number\tcount\n4999998\t1\n4999999\t1\n",
    );
}

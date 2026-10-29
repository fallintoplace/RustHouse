use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn billion_row_count_groups_support_descending_pages() {
    assert_eq!(
        run("SELECT number, count(*) FROM numbers(1000000000) GROUP BY number ORDER BY number DESC LIMIT 2 OFFSET 1"),
        "number\tcount\n999999998\t1\n999999997\t1\n"
    );
    assert_eq!(
        run("SELECT number, count(*) FROM numbers(1000000000) WHERE number >= 999999995 GROUP BY number ORDER BY number ASC LIMIT 2 OFFSET 1"),
        "number\tcount\n999999996\t1\n999999997\t1\n"
    );
}

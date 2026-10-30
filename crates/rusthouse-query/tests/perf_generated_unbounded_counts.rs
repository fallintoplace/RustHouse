use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn streams_unbounded_unique_count_groups_in_input_order() {
    assert_eq!(
        run("SELECT number, count(*) FROM numbers(4) GROUP BY number"),
        "number\tcount\n0\t1\n1\t1\n2\t1\n3\t1\n"
    );
    assert_eq!(
        run("SELECT number, count(*) FROM numbers(4) GROUP BY number OFFSET 2"),
        "number\tcount\n2\t1\n3\t1\n"
    );
    assert_eq!(
        run("SELECT number, count(*) FROM numbers(0) GROUP BY number"),
        "number\tcount\n"
    );
}

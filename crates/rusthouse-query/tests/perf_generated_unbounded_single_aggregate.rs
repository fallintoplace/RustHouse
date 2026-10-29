use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn streams_generated_single_metric_groups_without_hashing() {
    assert_eq!(
        run("SELECT number, sum(number) FROM numbers(4) GROUP BY number"),
        "number\tsum(number)\n0\t0\n1\t1\n2\t2\n3\t3\n"
    );
    assert_eq!(
        run("SELECT number, max(number) FROM numbers(4) GROUP BY number OFFSET 2"),
        "number\tmax(number)\n2\t2\n3\t3\n"
    );
}

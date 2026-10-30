use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn streams_unbounded_generated_multi_aggregate_rows() {
    assert_eq!(
        run("SELECT number, sum(number), max(number) FROM numbers(3) GROUP BY number"),
        "number\tsum(number)\tmax(number)\n0\t0\t0\n1\t1\t1\n2\t2\t2\n"
    );
    assert_eq!(
        run("SELECT number, sum(number), count(number) FROM numbers(3) GROUP BY number OFFSET 1"),
        "number\tsum(number)\tcount(number)\n1\t1\t1\n2\t2\t1\n"
    );
}

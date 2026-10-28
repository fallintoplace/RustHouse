use rusthouse_query::{execute, explain_select, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn generates_inclusive_ranges_and_steps() {
    assert_eq!(
        run("SELECT number FROM generate_series(5, 11, 3)"),
        "number\n5\n8\n11\n"
    );
    assert_eq!(
        run("SELECT count(*) FROM generate_series(3, 9, 2)"),
        "count\n4\n"
    );
    assert_eq!(run("SELECT number FROM generate_series(11, 5)"), "number\n");
    assert_eq!(
        run("SELECT number FROM generate_series(0, 0)"),
        "number\n0\n"
    );
}

#[test]
fn handles_u64_boundary_without_overflow() {
    let max = u64::MAX;
    assert_eq!(
        run(&format!("SELECT number FROM generate_series({max}, {max})")),
        format!("number\n{max}\n")
    );
    assert_eq!(
        run(&format!(
            "SELECT number FROM generate_series(0, {max}, {max})"
        )),
        format!("number\n0\n{max}\n")
    );
}

#[test]
fn uses_existing_filters_sorting_and_pagination() {
    assert_eq!(
        run(
            "SELECT number FROM generate_series(2, 10, 2) WHERE number > 4 ORDER BY number DESC LIMIT 2"
        ),
        "number\n10\n8\n"
    );
    let query = parse_select("SELECT number FROM generate_series(5, 11, 3)").unwrap();
    assert!(explain_select(&query).contains("generate_series(5, 11, 3)"));
}

#[test]
fn rejects_invalid_arguments_and_unbounded_queries() {
    for sql in [
        "SELECT number FROM generate_series(1)",
        "SELECT number FROM generate_series(1, 10, 0)",
        "SELECT number FROM generate_series(0, 10000000)",
        "SELECT number FROM generate_series(0, 18446744073709551615)",
        "SELECT number FROM generate_series(0, 5, 1, 2)",
        "SELECT number FROM generate_series(-1, 5)",
        "SELECT number FROM generate_series(0, 18446744073709551616)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn rejects_part_file_with_generated_series() {
    let query = parse_select("SELECT number FROM generate_series(1, 2)").unwrap();
    let error = execute(&query, Some(std::path::Path::new(".")), &mut Vec::new())
        .unwrap_err()
        .to_string();
    assert!(error.contains("--part cannot be used"), "{error}");
}

use rusthouse_query::{execute, parse_select};

fn query(sql: &str) -> Result<String, String> {
    let select = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&select, None, &mut output).map_err(|error| error.to_string())?;
    Ok(String::from_utf8(output).unwrap())
}

#[test]
fn count_if_counts_nonzero_flags() {
    assert_eq!(
        query("SELECT countIf(number) FROM numbers(5)").unwrap(),
        "countIf(number)\n4\n"
    );
    assert_eq!(
        query("SELECT countIf(number) FROM numbers(5) WHERE number >= 3").unwrap(),
        "countIf(number)\n2\n"
    );
    assert_eq!(
        query("SELECT number, countIf(number) FROM numbers(3) GROUP BY number").unwrap(),
        "number\tcountIf(number)\n0\t0\n1\t1\n2\t1\n"
    );
}

#[test]
fn count_if_returns_zero_for_empty_sets_and_respects_limits() {
    assert_eq!(
        query("SELECT countIf(number) FROM numbers(0)").unwrap(),
        "countIf(number)\n0\n"
    );
    assert_eq!(
        query("SELECT countIf(number) FROM numbers(1)").unwrap(),
        "countIf(number)\n0\n"
    );
    assert_eq!(
        query("SELECT countIf(number) FROM numbers(3) LIMIT 0").unwrap(),
        "countIf(number)\n"
    );
}

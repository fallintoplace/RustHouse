use rusthouse_query::{execute, parse_select};

fn query(sql: &str) -> Result<String, String> {
    let select = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&select, None, &mut output).map_err(|error| error.to_string())?;
    Ok(String::from_utf8(output).unwrap())
}

#[test]
fn range_supports_generated_sequences_and_filters() {
    assert_eq!(
        query("SELECT range(number) FROM numbers(2, 4)").unwrap(),
        "range(number)\n3\n"
    );
    assert_eq!(
        query("SELECT range(number) FROM generate_series(2, 10, 2)").unwrap(),
        "range(number)\n8\n"
    );
    assert_eq!(
        query("SELECT range(number) FROM numbers(5) WHERE number >= 3").unwrap(),
        "range(number)\n1\n"
    );
}

#[test]
fn range_handles_singletons_empty_and_u64_boundaries() {
    assert_eq!(
        query("SELECT range(number) FROM numbers(0)").unwrap(),
        "range(number)\nNULL\n"
    );
    assert_eq!(
        query("SELECT range(number) FROM numbers(18446744073709551614, 2)").unwrap(),
        "range(number)\n1\n"
    );
    assert_eq!(
        query("SELECT number, range(number) FROM numbers(3) GROUP BY number").unwrap(),
        "number\trange(number)\n0\t0\n1\t0\n2\t0\n"
    );
}

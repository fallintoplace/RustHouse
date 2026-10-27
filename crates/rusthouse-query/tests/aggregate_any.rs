use rusthouse_query::{execute, parse_select};

fn query(sql: &str) -> Result<String, String> {
    let select = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&select, None, &mut output).map_err(|error| error.to_string())?;
    Ok(String::from_utf8(output).unwrap())
}

#[test]
fn any_returns_first_non_null_value_in_scan_order() {
    assert_eq!(
        query("SELECT any(number) FROM numbers(5)").unwrap(),
        "any(number)\n0\n"
    );
    assert_eq!(
        query("SELECT any(number) FROM numbers(5) WHERE number >= 2").unwrap(),
        "any(number)\n2\n"
    );
    assert_eq!(
        query("SELECT number, any(number) FROM numbers(3) GROUP BY number").unwrap(),
        "number\tany(number)\n0\t0\n1\t1\n2\t2\n"
    );
}

#[test]
fn any_returns_null_for_empty_sets() {
    assert_eq!(
        query("SELECT any(number) FROM numbers(0)").unwrap(),
        "any(number)\nNULL\n"
    );
    assert_eq!(
        query("SELECT any(number) FROM numbers(3) LIMIT 0").unwrap(),
        "any(number)\n"
    );
}

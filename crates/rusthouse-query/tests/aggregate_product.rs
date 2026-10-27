use rusthouse_query::{execute, parse_select};

fn query(sql: &str) -> Result<String, String> {
    let select = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&select, None, &mut output).map_err(|error| error.to_string())?;
    Ok(String::from_utf8(output).unwrap())
}

#[test]
fn product_supports_scans_filters_and_groups() {
    assert_eq!(query("SELECT product(number) FROM numbers(1, 4)").unwrap(), "product(number)\n24\n");
    assert_eq!(query("SELECT product(number) FROM numbers(5) WHERE number >= 3").unwrap(), "product(number)\n12\n");
    assert_eq!(
        query("SELECT number, product(number) FROM numbers(3) GROUP BY number ORDER BY number").unwrap(),
        "number\tproduct(number)\n0\t0\n1\t1\n2\t2\n"
    );
}

#[test]
fn product_handles_empty_and_overflow() {
    assert_eq!(query("SELECT product(number) FROM numbers(0)").unwrap(), "product(number)\nNULL\n");
    assert_eq!(query("SELECT product(number) FROM numbers(2) LIMIT 0").unwrap(), "product(number)\n");
    assert!(query("SELECT product(number) FROM numbers(1, 41)").unwrap_err().contains("PRODUCT overflow"));
}

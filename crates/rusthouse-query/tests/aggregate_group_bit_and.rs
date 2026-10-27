use rusthouse_query::{execute, parse_select};

fn query(sql: &str) -> Result<String, String> {
    let select = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&select, None, &mut output).map_err(|error| error.to_string())?;
    Ok(String::from_utf8(output).unwrap())
}

#[test]
fn group_bit_and_reduces_integer_flags() {
    assert_eq!(
        query("SELECT groupBitAnd(number) FROM numbers(6, 2)").unwrap(),
        "groupBitAnd(number)\n6\n"
    );
    assert_eq!(
        query("SELECT groupBitAnd(number) FROM numbers(8) WHERE number >= 6").unwrap(),
        "groupBitAnd(number)\n6\n"
    );
    assert_eq!(
        query("SELECT number, groupBitAnd(number) FROM numbers(3) GROUP BY number").unwrap(),
        "number\tgroupBitAnd(number)\n0\t0\n1\t1\n2\t2\n"
    );
}

#[test]
fn group_bit_and_supports_empty_and_large_inputs() {
    assert_eq!(
        query("SELECT groupBitAnd(number) FROM numbers(0)").unwrap(),
        "groupBitAnd(number)\nNULL\n"
    );
    assert_eq!(
        query("SELECT groupBitAnd(number) FROM numbers(18446744073709551614, 2)").unwrap(),
        "groupBitAnd(number)\n18446744073709551614\n"
    );
}

use rusthouse_query::{execute, parse_select};

fn result(sql: &str) -> Vec<u8> {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    output
}
#[test]
fn variance_uses_sample_variance() {
    for (sql, canonical) in [
        ("SELECT VARIANCE(number) FROM numbers(4)", "SELECT varSamp(number) FROM numbers(4)"),
        ("SELECT VARIANCE(number), COUNT(*) FROM numbers(4)",
         "SELECT varSamp(number), COUNT(*) FROM numbers(4)"),
        ("SELECT number, VARIANCE(number) FROM numbers(4) GROUP BY number",
         "SELECT number, varSamp(number) FROM numbers(4) GROUP BY number"),
    ] {
        assert_eq!(parse_select(sql).unwrap(), parse_select(canonical).unwrap());
    }
    assert_eq!(
        result("SELECT VARIANCE(number) FROM numbers(4)"),
        result("SELECT varSamp(number) FROM numbers(4)")
    );
}

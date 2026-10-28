use rusthouse_query::{execute, parse_select};

#[test]
fn generate_series_accepts_larger_ranges_and_keeps_a_bound() {
    let query = parse_select("SELECT count(*) FROM generate_series(1, 10000000)").unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    assert_eq!(output, b"count\n10000000\n");
    assert!(parse_select("SELECT number FROM generate_series(1, 10000001)").is_err());
    // A tiny number of stepped values is still legal near UInt64::MAX.
    assert!(parse_select(
        "SELECT number FROM generate_series(18446744073709551614, 18446744073709551615)"
    ).is_ok());
}

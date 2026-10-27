use rusthouse_query::{execute, parse_select};

#[test]
fn nested_scalar_expressions_return_expected_values() {
    for (sql, expected) in [
        (
            "SELECT GREATEST(-17, 9 * 3, (8 - 25)) AS largest",
            "largest\n27\n",
        ),
        (
            "SELECT FLOOR_DIV(-29, 5) AS rounded",
            "rounded\n-6\n",
        ),
        (
            "SELECT LCM(GCD(84, 30), 15) AS factor",
            "factor\n30\n",
        ),
        (
            "SELECT bitCount(bitShiftLeft(3, 4)) AS bits",
            "bits\n2\n",
        ),
    ] {
        let parsed = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
        let mut output = Vec::new();
        execute(&parsed, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{sql}");
    }
}

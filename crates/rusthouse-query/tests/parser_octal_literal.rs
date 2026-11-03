use rusthouse_query::{parse_select, Projection};

#[test]
fn octal_literals_use_checked_integer_scanner() {
    assert!(matches!(
        parse_select("SELECT 0o17").unwrap().projection,
        Projection::Literal(15)
    ));
    assert!(matches!(
        parse_select("SELECT 0O10").unwrap().projection,
        Projection::Literal(8)
    ));
    for invalid in ["SELECT 0o", "SELECT 0o8", "SELECT 0o9", "SELECT 0o777777777777777777777777777777"] {
        assert!(parse_select(invalid).is_err(), "{invalid}");
    }
}

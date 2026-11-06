use super::*;

#[test]
fn compares_mixed_values_without_losing_precision() {
    let unsigned = |value| Cell::Numeric(Number::Unsigned(value));
    let signed = |value| Cell::Numeric(Number::Signed(value));
    let float = |value| Cell::Numeric(Number::Float(value));
    assert!(matches_comparison(&unsigned(0), &signed(-1), Operator::Gt));
    assert!(matches_comparison(&signed(-1), &unsigned(0), Operator::Lt));
    assert!(matches_comparison(
        &unsigned((1 << 53) + 1),
        &float((1u64 << 53) as f64),
        Operator::Gt
    ));
    assert!(matches_comparison(
        &unsigned(u128::MAX),
        &float(f64::INFINITY),
        Operator::Lt
    ));
    assert!(!matches_comparison(
        &unsigned(1),
        &float(f64::NAN),
        Operator::Eq
    ));
    assert!(matches_comparison(
        &unsigned(1),
        &float(f64::NAN),
        Operator::Ne
    ));
    assert!(!matches_comparison(&Cell::Null, &signed(-1), Operator::Ne));
}

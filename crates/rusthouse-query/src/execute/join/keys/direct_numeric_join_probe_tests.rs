use super::*;
use rusthouse_storage::NumericColumn;

#[test]
fn native_probe_keys_match_materialized_build_keys() {
    let batch = [
        TypedColumn::Numeric(NumericColumn::Int64(vec![-12, 9])),
        TypedColumn::NullableNumeric {
            values: NumericColumn::UInt64(vec![7, 8]),
            nulls: vec![0, 1],
        },
    ];
    let mut key = Vec::new();
    assert!(batch_key_into(&batch, 0, &[0, 1], &mut key));
    let row = [typed::Cell::Numeric(Number::Signed(-12)),
               typed::Cell::Numeric(Number::Unsigned(7))];
    assert_eq!(key, row_key(&row, &[0, 1]).unwrap());
    assert!(!batch_key_into(&batch, 1, &[0, 1], &mut key));
    assert!(key.is_empty());
    let nan = [TypedColumn::Numeric(NumericColumn::Float64(vec![f64::NAN]))];
    assert!(!batch_key_into(&nan, 0, &[0], &mut key));
}

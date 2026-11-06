use super::*;

#[test]
fn compiled_byte_bitmap_respects_nullable_and_signed_membership() {
    let unsigned = (0..20).map(|value| value.to_string()).collect::<Vec<_>>();
    let includes = [vec![Condition {
        index: 0,
        test: compile_in("Nullable(UInt8)", &unsigned, false).unwrap(),
    }]];
    assert!(matches!(&includes[0][0].test, Test::ByteInSet(_, false)));
    let batch = [TypedColumn::NullableNumeric {
        values: NumericColumn::UInt8(vec![0, 19, 20, 5]),
        nulls: vec![0, 0, 0, 1],
    }];
    assert!(passes(&batch, 0, &includes));
    assert!(passes(&batch, 1, &includes));
    assert!(!passes(&batch, 2, &includes));
    assert!(!passes(&batch, 3, &includes));
    let signed = (-20..0).map(|value| value.to_string()).collect::<Vec<_>>();
    let excludes = [vec![Condition {
        index: 0,
        test: compile_in("Int8", &signed, true).unwrap(),
    }]];
    let batch = [TypedColumn::Numeric(NumericColumn::Int8(vec![-20, -1, 1]))];
    assert!(!passes(&batch, 0, &excludes));
    assert!(!passes(&batch, 1, &excludes));
    assert!(passes(&batch, 2, &excludes));
}

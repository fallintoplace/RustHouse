use super::*;

#[test]
fn compiled_byte_comparisons_match_generic_on_signed_unsigned_and_nulls() {
    for kind in ["UInt8", "Int8"] {
        let indices = HashMap::from([("status".to_owned(), 0usize)]);
        let types = HashMap::from([("status".to_owned(), format!("Nullable({kind})"))]);
        for op in [
            Operator::Eq,
            Operator::Ne,
            Operator::Lt,
            Operator::Le,
            Operator::Gt,
            Operator::Ge,
        ] {
            let predicate = Predicate::Comparison {
                column: "status".into(),
                operator: op,
                value: "0".into(),
            };
            let conditions = compile(&[predicate], &indices, &types).unwrap();
            assert!(matches!(conditions[0].test, Test::ByteCompare(..)));
            let values = if kind == "Int8" {
                NumericColumn::Int8(vec![-128, -1, 0, 127])
            } else {
                NumericColumn::UInt8(vec![0, 1, 127, 255])
            };
            let batch = [TypedColumn::NullableNumeric {
                values,
                nulls: vec![0, 0, 1, 0],
            }];
            let target = Cell::Numeric(if kind == "Int8" {
                Number::Signed(0)
            } else {
                Number::Unsigned(0)
            });
            let old = vec![Condition {
                index: 0,
                test: Test::Compare(op, target),
            }];
            for row in 0..4 {
                assert_eq!(
                    passes(&batch, row, std::slice::from_ref(&conditions)),
                    passes(&batch, row, std::slice::from_ref(&old)),
                    "{kind}, {op:?}, row {row}"
                );
            }
        }
    }
}

use super::*;
use rusthouse_storage::{NumericColumn, TypedColumn, read_legacy_strings};

#[test]
fn reordering_mixed_nullable_predicates_preserves_every_row_result() {
    let mut input = b"\x05alpha\x04beta\x05gamma".as_slice();
    let text = read_legacy_strings(&mut input, 3).unwrap();
    let batch = [
        TypedColumn::Text(text),
        TypedColumn::NullableNumeric {
            values: NumericColumn::UInt64(vec![1, 2, 3]),
            nulls: vec![0, 1, 0],
        },
    ];
    let mut conditions = vec![
        Condition {
            index: 0,
            test: Test::Like {
                pattern: compile_like("a%").unwrap(),
                case_insensitive: false,
                negated: false,
            },
        },
        Condition {
            index: 1,
            test: Test::Compare(Operator::Eq, Cell::Numeric(Number::Unsigned(1))),
        },
        Condition {
            index: 1,
            test: Test::IsNull(true),
        },
    ];
    let expected = (0..3)
        .map(|row| passes(&batch, row, std::slice::from_ref(&conditions)))
        .collect::<Vec<_>>();
    assert_eq!(expected, vec![true, false, false]);
    order_compiled_conditions(&mut conditions);
    assert!(matches!(conditions[0].test, Test::IsNull(true)));
    assert!(matches!(conditions[1].test, Test::Compare(..)));
    assert!(matches!(conditions[2].test, Test::Like { .. }));
    let actual = (0..3)
        .map(|row| passes(&batch, row, std::slice::from_ref(&conditions)))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn modulos_and_wildcards_follow_cheap_bitmap_checks() {
    let mut conditions = vec![
        Condition {
            index: 0,
            test: Test::Like {
                pattern: compile_like("%needle%").unwrap(),
                case_insensitive: true,
                negated: false,
            },
        },
        Condition {
            index: 1,
            test: Test::Compare(Operator::Modulo(3), Cell::Numeric(Number::Unsigned(2))),
        },
        Condition {
            index: 2,
            test: Test::IsNull(false),
        },
        Condition {
            index: 3,
            test: Test::ByteInSet([0; 4], false),
        },
    ];
    order_compiled_conditions(&mut conditions);
    assert!(matches!(conditions[0].test, Test::IsNull(false)));
    assert!(matches!(conditions[1].test, Test::ByteInSet(..)));
    assert!(matches!(
        conditions[2].test,
        Test::Compare(Operator::Modulo(_), _)
    ));
    assert!(matches!(conditions[3].test, Test::Like { .. }));
}

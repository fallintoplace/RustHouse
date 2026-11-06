use super::*;

#[test]
fn nan_is_unordered_not_outside_an_interval() {
    let range = CompiledTest::NotBetween {
        lower: Number::Float(-1.0),
        upper: Number::Float(1.0),
    };
    assert!(!range.matches(Number::Float(f64::NAN)));
    assert!(!range.matches(Number::Float(0.0)));
    assert!(range.matches(Number::Float(3.0)));
    let invalid = CompiledTest::NotBetween {
        lower: Number::Float(f64::NAN),
        upper: Number::Float(3.0),
    };
    assert!(!invalid.matches(Number::Float(7.0)));
}

#[test]
fn vectorized_not_between_agrees_with_unordered_rowwise_semantics() {
    let column = NumericColumn::Float64(vec![f64::NAN, -2.0, 0.0, 2.0]);
    let branches = vec![vec![CompiledPredicate {
        index: 0,
        test: CompiledTest::NotBetween {
            lower: Number::Float(-1.0),
            upper: Number::Float(1.0),
        },
    }]];
    assert_eq!(
        count_batch_matches(std::slice::from_ref(&column), &branches),
        2
    );
    assert_eq!(
        (0..column.len())
            .filter(|&i| { batch_matches(std::slice::from_ref(&column), i, &branches) })
            .count(),
        2
    );
}

use super::*;

fn count(start: u64, rows: u64, modulus: u64, remainder: u128, negated: bool) -> u64 {
    let predicate = CompiledPredicate {
        index: 0,
        test: CompiledTest::Comparison {
            operator: if negated {
                Operator::ModuloNe(modulus)
            } else {
                Operator::Modulo(modulus)
            },
            target: Number::Unsigned(remainder),
        },
    };
    count_monotonic_numbers(&[vec![predicate]], start, rows, 1).unwrap()
}

#[test]
fn modulo_count_skips_billion_row_iteration() {
    assert_eq!(count(0, 1_000_000_000, 7, 0, false), 142_857_143);
    assert_eq!(count(0, 1_000_000_000, 7, 0, true), 857_142_857);
}

#[test]
fn modulo_count_handles_offsets_and_impossible_remainders() {
    assert_eq!(count(10, 20, 7, 0, false), 3);
    assert_eq!(count(10, 20, 7, 0, true), 17);
    assert_eq!(count(10, 20, 7, 7, false), 0);
    assert_eq!(count(10, 20, 7, 7, true), 20);
    assert_eq!(count(0, 0, 7, 0, false), 0);
}

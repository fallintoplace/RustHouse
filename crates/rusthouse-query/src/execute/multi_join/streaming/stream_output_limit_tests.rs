use super::super::super::Number;
use super::*;

#[test]
fn streaming_inner_joins_can_emit_more_than_the_materialized_row_cap() {
    let fact = typed::Cell::Numeric(Number::Unsigned(7));
    let dimension = typed::Cell::Numeric(Number::Unsigned(7));
    let right_rows = vec![vec![dimension]];
    let mut hash = HashMap::new();
    hash.insert(row_key(&right_rows[0], &[0]).unwrap(), Bucket::One(0));
    let stages = [Stage {
        rows: right_rows,
        hash,
        left_keys: vec![0],
        any: false,
    }];
    let selected = [(0, "fact.id".to_owned(), None)];
    let spec = OutputSpec {
        selected: &selected,
        types: vec!["UInt64"],
        null_filter: None,
        comparisons: &[],
        column_comparisons: &[],
        group_indices: None,
        group_year: None,
        sum: None,
        scalar_sum: false,
        grouped_min: false,
        formula: None,
        grouped_formula: None,
        distinct_index: None,
        row_filters: None,
        exists: &[],
        avg: None,
        correlated_sum: None,
        offset: 0,
        limit: None,
    };
    let mut output = std::io::sink();
    let mut state = OutputState::default();
    let mut row = vec![fact];
    let mut scratch_keys = (0..stages.len()).map(|_| Vec::new()).collect::<Vec<_>>();
    for _ in 0..131_100 {
        visit(
            0,
            &mut row,
            &stages,
            &mut scratch_keys,
            &spec,
            &mut state,
            &mut output,
        )
        .unwrap();
    }
    assert_eq!(state.seen, 131_100);
    assert_eq!(state.emitted, 131_100);
    assert_eq!(row.len(), 1);
}

#[test]
fn streaming_inner_join_still_respects_explicit_limit() {
    let fact = typed::Cell::Numeric(Number::Unsigned(7));
    let dimension = typed::Cell::Numeric(Number::Unsigned(7));
    let right_rows = vec![vec![dimension]];
    let mut hash = HashMap::new();
    hash.insert(row_key(&right_rows[0], &[0]).unwrap(), Bucket::One(0));
    let stages = [Stage {
        rows: right_rows,
        hash,
        left_keys: vec![0],
        any: false,
    }];
    let selected = [(0, "fact.id".to_owned(), None)];
    let spec = OutputSpec {
        selected: &selected,
        types: vec!["UInt64"],
        null_filter: None,
        comparisons: &[],
        column_comparisons: &[],
        group_indices: None,
        group_year: None,
        sum: None,
        scalar_sum: false,
        grouped_min: false,
        formula: None,
        grouped_formula: None,
        distinct_index: None,
        row_filters: None,
        exists: &[],
        avg: None,
        correlated_sum: None,
        offset: 0,
        limit: Some(2),
    };
    let mut state = OutputState::default();
    let mut row = vec![fact];
    let mut output = std::io::sink();
    let mut scratch_keys = (0..stages.len()).map(|_| Vec::new()).collect::<Vec<_>>();
    for _ in 0..100 {
        visit(
            0,
            &mut row,
            &stages,
            &mut scratch_keys,
            &spec,
            &mut state,
            &mut output,
        )
        .unwrap();
    }
    assert_eq!(state.emitted, 2);
}

#[test]
fn grouped_sum_orders_mixed_full_width_values_numerically() {
    use std::cmp::Ordering;
    let negative = Some(Number::Signed(-1));
    let large = Some(Number::Unsigned(u128::MAX));
    assert_eq!(
        compare_sum_values(negative, large, false, None),
        Ordering::Less
    );
    assert_eq!(
        compare_sum_values(negative, large, true, None),
        Ordering::Greater
    );
    assert_eq!(
        compare_sum_values(None, large, false, Some(false)),
        Ordering::Greater
    );
}

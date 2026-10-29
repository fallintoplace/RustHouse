use super::*;
use super::super::super::Number;

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
        null_filter: None,
        offset: 0,
        limit: None,
    };
    let mut output = std::io::sink();
    let mut state = OutputState::default();
    let mut row = vec![fact];
    for _ in 0..131_100 {
        visit(0, &mut row, &stages, &spec, &mut state, &mut output).unwrap();
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
        null_filter: None,
        offset: 0,
        limit: Some(2),
    };
    let mut state = OutputState::default();
    let mut row = vec![fact];
    let mut output = std::io::sink();
    for _ in 0..100 {
        visit(0, &mut row, &stages, &spec, &mut state, &mut output).unwrap();
    }
    assert_eq!(state.emitted, 2);
}

use super::*;

#[test]
fn compares_borrowed_text_without_replacing_ties() {
    let incumbent = Cell::Text(b"beta".to_vec());
    assert!(replaces_text_extreme(
        b"alpha",
        Some(&incumbent),
        AggregateFunction::Min
    ));
    assert!(!replaces_text_extreme(
        b"beta",
        Some(&incumbent),
        AggregateFunction::Min
    ));
    assert!(!replaces_text_extreme(
        b"alpha",
        Some(&incumbent),
        AggregateFunction::Max
    ));
    assert!(replaces_text_extreme(
        b"gamma",
        Some(&incumbent),
        AggregateFunction::Max
    ));
    assert!(replaces_text_extreme(b"beta", None, AggregateFunction::Min));
}

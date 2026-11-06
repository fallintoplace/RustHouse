use crate::parse_select;

#[test]
fn explicit_asymmetric_uses_the_default_between_semantics() {
    for modifier in ["", "NOT "] {
        let plain = parse_select(&format!(
            "SELECT * FROM numbers(10) WHERE number {modifier}BETWEEN 2 AND 5"
        ))
        .unwrap();
        let explicit = parse_select(&format!(
            "SELECT * FROM numbers(10) WHERE number {modifier}BETWEEN ASYMMETRIC 2 AND 5"
        ))
        .unwrap();
        assert_eq!(plain.predicates, explicit.predicates);
        assert_eq!(plain.or_predicates, explicit.or_predicates);
    }
    assert!(
        parse_select("SELECT * FROM numbers(10) WHERE number BETWEEN ASYMMETRIC 1").is_err()
    );
}

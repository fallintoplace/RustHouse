use super::*;

#[test]
fn rejects_ambiguous_numeric_weights_before_manifest_serialization() {
    for weight in ["+1", "01", "0001", "-1", "1.0", "0", "1001", "١"] {
        let manifest = format!("node zone {weight}\n");
        assert!(parse_membership(&manifest).is_err(), "weight={weight}");
    }
    for weight in [1, 9, 10, 100, 1000] {
        let manifest = format!("node zone {weight}\n");
        let members = parse_membership(&manifest).unwrap();
        assert_eq!(format_membership(&members).unwrap(), manifest);
    }
}

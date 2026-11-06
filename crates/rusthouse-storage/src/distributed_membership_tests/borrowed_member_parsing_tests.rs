use super::*;

#[test]
fn parses_full_membership_without_duplicating_or_losing_values() {
    let manifest = (0..64)
        .map(|index| format!("n{index} zone{} {}\n", index % 8, index + 1))
        .collect::<String>();
    let parsed = parse_membership(&manifest).unwrap();
    assert_eq!(parsed.len(), 64);
    assert_eq!(format_membership(&parsed).unwrap(), manifest);
    assert!(parse_membership(&(manifest.clone() + "overflow zone0 1\n")).is_err());
    assert!(parse_membership(&(manifest + "n0 zone1 1\n")).is_err());
}

#[test]
fn preserves_validation_with_irregular_whitespace() {
    let members = parse_membership("n1\tzone1\t23\nn2   zone2  7\n").unwrap();
    assert_eq!(members[0].node, "n1");
    assert_eq!(members[0].weight, 23);
    assert_eq!(members[1].zone, "zone2");
    assert_eq!(
        format_membership(&members).unwrap(),
        "n1 zone1 23\nn2 zone2 7\n"
    );
}

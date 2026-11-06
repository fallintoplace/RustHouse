use super::*;

#[test]
fn validation_size_matches_wire_output_for_long_member_names() {
    for count in [1, 8, 9, 32, 64] {
        let members = (0..count)
            .map(|index| Member {
                node: format!("replica_{index:02}_{}", "n".repeat(40)),
                zone: format!("zone_{}", index % 7),
                weight: [1, 9, 10, 999, 1000][index % 5],
            })
            .collect::<Vec<_>>();
        let size = serialized_membership_bytes(&members).unwrap();
        let formatted = format_membership(&members).unwrap();
        assert_eq!(formatted.len(), size);
        assert_eq!(parse_membership(&formatted).unwrap(), members);
    }
}

#[test]
fn sizing_does_not_skip_duplicate_or_oversized_validation() {
    let item = Member {
        node: "n".into(),
        zone: "z".into(),
        weight: 1,
    };
    assert!(serialized_membership_bytes(&[item.clone(), item]).is_err());
    assert!(serialized_membership_bytes(&[]).is_err());
}

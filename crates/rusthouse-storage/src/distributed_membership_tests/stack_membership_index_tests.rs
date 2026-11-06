use super::*;

#[test]
fn full_membership_roundtrips_without_losing_duplicate_detection() {
    for size in [1usize, 2, 8, 9, 16, 32, 64] {
        let records = (0..size)
            .rev()
            .map(|index| format!("node_{index:02} zone_{} {}\n", index % 7, index % 1000 + 1))
            .collect::<String>();
        let members = parse_membership(&records).unwrap();
        assert_eq!(members.len(), size);
        assert_eq!(
            serialized_membership_bytes(&members).unwrap(),
            records.len()
        );
        assert_eq!(format_membership(&members).unwrap(), records);
        let mut duplicates = members.clone();
        if size == 1 {
            let repeated = duplicates[0].clone();
            duplicates.push(repeated);
        } else {
            duplicates[size - 1].node = duplicates[0].node.clone();
        }
        assert!(serialized_membership_bytes(&duplicates).is_err());
        let mut duplicate_text = records.clone();
        duplicate_text.push_str("node_00 zone_extra 1\n");
        assert!(parse_membership(&duplicate_text).is_err());
    }
}

#[test]
fn exhausted_index_rejects_new_names_and_old_duplicates() {
    let text = (0..64)
        .map(|index| format!("node_{index:02} zone_{} 1\n", index % 4))
        .collect::<String>();
    assert_eq!(parse_membership(&text).unwrap().len(), 64);
    assert!(parse_membership(&(text.clone() + "new_node zone_0 1\n")).is_err());
    assert!(parse_membership(&(text + "node_00 zone_0 1\n")).is_err());
}

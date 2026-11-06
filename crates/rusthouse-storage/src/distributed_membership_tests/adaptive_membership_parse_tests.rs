use super::*;

#[test]
fn validates_duplicates_across_the_stack_to_hash_transition() {
    let mut text = String::new();
    for index in 0..8 {
        text.push_str(&format!("node{index} zone0 1\n"));
    }
    assert_eq!(parse_membership(&text).unwrap().len(), 8);
    let mut duplicate = text.clone();
    duplicate.push_str("node0 zone1 2\n");
    assert!(parse_membership(&duplicate).is_err());
    let mut valid = text;
    valid.push_str("node8 zone1 2\n");
    assert_eq!(parse_membership(&valid).unwrap().len(), 9);
    valid.push_str("node8 zone0 1\n");
    assert!(parse_membership(&valid).is_err());
}

#[test]
fn full_membership_has_unique_names_at_the_record_limit() {
    let text = (0..64)
        .map(|index| format!("node{index} zone{} 1\n", index % 4))
        .collect::<String>();
    assert_eq!(parse_membership(&text).unwrap().len(), 64);
}

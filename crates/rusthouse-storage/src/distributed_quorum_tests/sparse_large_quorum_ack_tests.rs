use super::*;

#[test]
fn sparse_acks_match_full_member_count_and_preserve_errors() {
    let nodes = (0..16)
        .map(|index| format!("replica_{index}"))
        .collect::<Vec<_>>();
    let names = nodes.iter().map(String::as_str).collect::<Vec<_>>();
    let policy = QuorumPolicy::new(16, 8, 9).unwrap();
    for count in [0, 1, 2, 7, 8, 9, 16] {
        let observed = policy.observe(&names, &names[..count]).unwrap();
        assert_eq!(observed.distinct_acks, count);
        assert_eq!(observed.can_read, count >= 8);
        assert_eq!(observed.can_write, count >= 9);
    }
    assert_eq!(
        policy
            .observe(&names, &["replica_0"; 8])
            .unwrap()
            .distinct_acks,
        1
    );
    assert!(policy.observe(&names, &["missing"]).is_err());
    let mut duplicate = names.clone();
    duplicate[15] = duplicate[0];
    assert!(policy.observe(&duplicate, &[]).is_err());
    assert!(policy.observe(&duplicate, &names[..9]).is_err());
}

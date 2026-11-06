use super::*;

#[test]
fn large_placement_memberships_remain_deterministic_and_duplicate_safe() {
    for count in [9usize, 16, 32, 64] {
        let nodes = (0..count)
            .rev()
            .map(|n| format!("node_{n:02}"))
            .collect::<Vec<_>>();
        let first = replicas_for("analytics", &nodes, count.min(3)).unwrap();
        let mut ascending = nodes.clone();
        ascending.reverse();
        assert_eq!(
            first,
            replicas_for("analytics", &ascending, count.min(3)).unwrap()
        );
        assert_eq!(
            replicas_for_many(&["analytics"], &nodes, count.min(3)).unwrap(),
            vec![first]
        );
        let mut duplicate = nodes.clone();
        duplicate[count - 1] = duplicate[0].clone();
        assert!(replicas_for("analytics", &duplicate, 1).is_err());
        let mut invalid = nodes;
        invalid[count - 1] = "../invalid".into();
        assert!(replicas_for("analytics", &invalid, 1).is_err());
    }
}

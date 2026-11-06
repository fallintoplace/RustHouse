use super::*;

#[test]
fn accepts_intersecting_quorums() {
    let policy = QuorumPolicy::new(3, 2, 2).unwrap();
    assert!(policy.can_write(2));
    assert!(!policy.can_read(1));
    assert!(!policy.can_write(4));
    assert_eq!(QuorumPolicy::new(5, 2, 4).unwrap().writes, 4);
}

#[test]
fn counts_distinct_known_acknowledgements_only() {
    let policy = QuorumPolicy::new(3, 2, 2).unwrap();
    let members = ["a", "b", "c"];
    let single = policy.observe(&members, &["a", "a", "a"]).unwrap();
    assert_eq!(single.distinct_acks, 1);
    assert!(!single.can_read);
    assert!(!single.can_write);
    let majority = policy.observe(&members, &["a", "b", "b"]).unwrap();
    assert_eq!(majority.distinct_acks, 2);
    assert!(majority.can_read);
    assert!(majority.can_write);
    assert!(policy.observe(&members, &["unknown"]).is_err());
    assert!(policy.observe(&["a", "a", "c"], &["a"]).is_err());
    assert!(policy.observe(&["a", "b"], &["a"]).is_err());
    assert!(policy.observe(&["a", "b", ""], &["a"]).is_err());
    assert!(policy.observe(&members, &vec!["a"; 257]).is_err());
}

#[test]
fn small_and_large_memberships_count_votes_without_duplicates() {
    for replicas in [1usize, 2, 3, 5, 8, 9, 16] {
        let names = (0..replicas)
            .map(|index| format!("node_{index}"))
            .collect::<Vec<_>>();
        let members = names.iter().map(String::as_str).collect::<Vec<_>>();
        let policy = QuorumPolicy::new(replicas, 1, replicas).unwrap();
        let all = policy.observe(&members, &members).unwrap();
        assert_eq!(all.distinct_acks, replicas);
        assert!(all.can_read && all.can_write);
        let repeated = vec![members[0]; 256];
        assert_eq!(
            policy.observe(&members, &repeated).unwrap().distinct_acks,
            1
        );
        assert!(policy.observe(&members, &["missing"]).is_err());
        let mut duplicates = members.clone();
        if replicas > 1 {
            duplicates[replicas - 1] = duplicates[0];
            assert!(policy.observe(&duplicates, &[]).is_err());
        }
    }
}

#[test]
fn refuses_non_intersecting_or_unbounded_policies() {
    assert!(QuorumPolicy::new(3, 1, 2).is_err());
    assert!(QuorumPolicy::new(4, 2, 2).is_err());
    assert!(QuorumPolicy::new(0, 0, 0).is_err());
    assert!(QuorumPolicy::new(65, 33, 33).is_err());
}
#[test]
fn all_accepted_quorum_policies_intersect() {
    for replicas in 1..=16 {
        for reads in 1..=replicas {
            for writes in 1..=replicas {
                let policy = QuorumPolicy::new(replicas, reads, writes);
                let safe = reads + writes > replicas && writes > replicas / 2;
                assert_eq!(policy.is_ok(), safe, "{replicas}:{reads}:{writes}");
                if let Ok(policy) = policy {
                    assert!(!policy.can_read(reads - 1));
                    assert!(policy.can_read(reads));
                    assert!(!policy.can_write(writes - 1));
                    assert!(policy.can_write(writes));
                    assert!(!policy.can_write(replicas + 1));
                }
            }
        }
    }
}

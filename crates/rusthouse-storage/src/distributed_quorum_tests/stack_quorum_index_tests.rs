use super::*;

#[test]
fn many_acknowledgements_match_reference_masks_in_every_large_group() {
    for replicas in 9..=16 {
        let names = (0..replicas)
            .map(|n| format!("member_{n}"))
            .collect::<Vec<_>>();
        let members = names.iter().map(String::as_str).collect::<Vec<_>>();
        let policy = QuorumPolicy::new(replicas, 1, replicas).unwrap();
        for seed in 0..32usize {
            let acks = (0..256)
                .map(|n| members[(n * 37 + seed) % replicas])
                .collect::<Vec<_>>();
            let reference = (0..replicas)
                .filter(|&i| acks.contains(&members[i]))
                .count();
            let observed = policy.observe(&members, &acks).unwrap();
            assert_eq!(observed.distinct_acks, reference);
            assert_eq!(observed.can_write, reference >= replicas);
        }
        let mut duplicates = members.clone();
        duplicates[replicas - 1] = duplicates[0];
        assert!(policy.observe(&duplicates, &members).is_err());
    }
}

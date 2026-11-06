use super::*;

#[test]
fn observations_match_checked_policy_for_all_quorum_thresholds() {
    for replicas in 1..=16 {
        let names = (0..replicas)
            .map(|index| format!("replica_{index}"))
            .collect::<Vec<_>>();
        let members = names.iter().map(String::as_str).collect::<Vec<_>>();
        for writes in 1..=replicas {
            for reads in 1..=replicas {
                let Ok(policy) = QuorumPolicy::new(replicas, reads, writes) else {
                    continue;
                };
                for ack_count in 0..=replicas {
                    let observed = policy.observe(&members, &members[..ack_count]).unwrap();
                    assert_eq!(observed.distinct_acks, ack_count);
                    assert_eq!(observed.can_read, policy.can_read(ack_count));
                    assert_eq!(observed.can_write, policy.can_write(ack_count));
                }
            }
        }
    }
}

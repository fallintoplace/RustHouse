use super::*;
use crate::distributed_membership::Member;

fn membership(zones: &[usize]) -> Vec<Member> {
    zones
        .iter()
        .enumerate()
        .flat_map(|(zone, &count)| {
            (0..count).map(move |index| Member {
                node: format!("node_{zone}_{index}"),
                zone: format!("zone_{zone}"),
                weight: 1,
            })
        })
        .collect()
}

#[test]
fn worst_case_matches_exhaustive_zone_failure_subsets() {
    for counts in [
        &[1usize, 1, 1][..],
        &[1, 2, 1, 1],
        &[4, 1, 2],
        &[2, 3, 1, 2, 1],
        &[3, 3, 3, 2, 3, 2],
    ] {
        let members = membership(counts);
        let n = members.len();
        let writes = n / 2 + 1;
        let reads = n - writes + 1;
        let policy = QuorumPolicy::new(n, reads, writes).unwrap();
        for failed in 0..=counts.len() {
            let mut most_lost = 0usize;
            for subset in 0..(1usize << counts.len()) {
                if subset.count_ones() as usize != failed {
                    continue;
                }
                let lost = counts
                    .iter()
                    .enumerate()
                    .filter(|(zone, _)| subset & (1 << zone) != 0)
                    .map(|(_, &size)| size)
                    .sum::<usize>();
                most_lost = most_lost.max(lost);
            }
            let expected = n - most_lost;
            let result = worst_case_zone_quorum(policy, &members, failed).unwrap();
            assert_eq!(
                result.surviving_nodes, expected,
                "zones={counts:?} failed={failed}"
            );
            assert_eq!(result.can_read, expected >= reads);
            assert_eq!(result.can_write, expected >= writes);
        }
    }
}

#[test]
fn rejects_bad_membership_and_out_of_range_zone_failures() {
    let policy = QuorumPolicy::new(3, 2, 2).unwrap();
    let members = membership(&[2, 1]);
    assert!(worst_case_zone_quorum(policy, &members, 3).is_err());
    assert_eq!(
        worst_case_zone_quorum(policy, &members, 0)
            .unwrap()
            .surviving_nodes,
        3
    );
    let wrong = membership(&[2, 2]);
    assert!(worst_case_zone_quorum(policy, &wrong, 1).is_err());
    let mut repeated = members.clone();
    repeated[2].node = repeated[0].node.clone();
    assert!(worst_case_zone_quorum(policy, &repeated, 0).is_err());
}

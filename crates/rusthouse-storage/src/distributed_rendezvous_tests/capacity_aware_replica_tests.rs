use super::*;
use crate::distributed_capacity::Capacity;
use crate::distributed_membership::parse_membership;

fn capacity(free: u64) -> Capacity {
    Capacity {
        total: 100,
        used: 100 - free,
        reserved: 0,
    }
}

#[test]
fn placement_matches_zone_rank_after_excluding_full_nodes() {
    let members = parse_membership("a z1 9\nb z2 4\nc z3 2\nd z1 1\n").unwrap();
    let capacities = [
        ("c", capacity(10)),
        ("a", capacity(1)),
        ("d", capacity(10)),
        ("b", capacity(10)),
    ];
    for shard in ["logs", "metrics", "events"] {
        let expected = zone_aware_replicas_for(shard, &members[1..], 3).unwrap();
        let chosen = capacity_aware_replicas_for(shard, &members, &capacities, 2, 3).unwrap();
        assert_eq!(chosen, expected);
        let all = [
            ("a", capacity(10)),
            ("b", capacity(10)),
            ("c", capacity(10)),
            ("d", capacity(10)),
        ];
        assert_eq!(
            capacity_aware_replicas_for(shard, &members, &all, 2, 2).unwrap(),
            zone_aware_replicas_for(shard, &members, 2).unwrap()
        );
    }
}

#[test]
fn rejects_unknown_duplicate_and_overcommitted_capacity_data() {
    let members = parse_membership("a z1 1\nb z2 1\n").unwrap();
    assert!(
        capacity_aware_replicas_for(
            "x",
            &members,
            &[("a", capacity(1)), ("b", capacity(1))],
            2,
            1
        )
        .is_err()
    );
    assert!(
        capacity_aware_replicas_for(
            "x",
            &members,
            &[("a", capacity(10)), ("a", capacity(10))],
            1,
            1
        )
        .is_err()
    );
    assert!(
        capacity_aware_replicas_for(
            "x",
            &members,
            &[("a", capacity(10)), ("unknown", capacity(10))],
            1,
            1
        )
        .is_err()
    );
    let invalid = Capacity {
        total: 10,
        used: 9,
        reserved: 2,
    };
    assert!(
        capacity_aware_replicas_for(
            "x",
            &members,
            &[("a", invalid), ("b", capacity(10))],
            1,
            1
        )
        .is_err()
    );
}

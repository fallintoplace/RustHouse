use super::*;
use crate::distributed_capacity::Capacity;
use crate::distributed_membership::Member;

#[test]
fn small_capacity_lookup_matches_validated_zone_placement() {
    for count in [2usize, 3, 8, 9, 16] {
        let members = (0..count)
            .map(|i| Member {
                node: format!("replica{i}"),
                zone: format!("zone{}", i % 3),
                weight: (i as u16 % 4) + 1,
            })
            .collect::<Vec<_>>();
        let slots = members
            .iter()
            .rev()
            .map(|member| {
                (
                    member.node.as_str(),
                    Capacity {
                        total: 100,
                        used: 10,
                        reserved: 5,
                    },
                )
            })
            .collect::<Vec<_>>();
        for replicas in 1..=count.min(4) {
            let expected = zone_aware_replicas_for("snapshot1", &members, replicas).unwrap();
            assert_eq!(
                capacity_aware_replicas_for("snapshot1", &members, &slots, 85, replicas)
                    .unwrap(),
                expected
            );
        }
    }
}

#[test]
fn small_capacity_lookup_rejects_duplicates_missing_and_invalid_states() {
    let members = vec![
        Member {
            node: "a".into(),
            zone: "z1".into(),
            weight: 1,
        },
        Member {
            node: "b".into(),
            zone: "z2".into(),
            weight: 1,
        },
    ];
    let valid = Capacity {
        total: 10,
        used: 0,
        reserved: 0,
    };
    assert!(
        capacity_aware_replicas_for("shard", &members, &[("a", valid), ("a", valid)], 1, 1,)
            .is_err()
    );
    assert!(
        capacity_aware_replicas_for(
            "shard",
            &members,
            &[("a", valid), ("foreign", valid)],
            1,
            1,
        )
        .is_err()
    );
    assert!(
        capacity_aware_replicas_for(
            "shard",
            &members,
            &[
                ("a", valid),
                (
                    "b",
                    Capacity {
                        total: 1,
                        used: 2,
                        reserved: 0
                    }
                )
            ],
            1,
            1,
        )
        .is_err()
    );
}

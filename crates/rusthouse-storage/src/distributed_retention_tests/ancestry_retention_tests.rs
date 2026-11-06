use super::*;

#[test]
fn never_selects_ancestors_of_the_active_or_pinned_generations() {
    let history = [
        GenerationParent {
            generation: 6,
            parent: Some(4),
        },
        GenerationParent {
            generation: 1,
            parent: None,
        },
        GenerationParent {
            generation: 4,
            parent: Some(2),
        },
        GenerationParent {
            generation: 5,
            parent: Some(3),
        },
        GenerationParent {
            generation: 3,
            parent: Some(1),
        },
        GenerationParent {
            generation: 2,
            parent: Some(1),
        },
    ];
    assert_eq!(
        retention_candidates_with_ancestry(&history, 6, &[5], 0).unwrap(),
        Vec::<u64>::new()
    );
    assert_eq!(
        retention_candidates_with_ancestry(&history, 6, &[], 0).unwrap(),
        [3, 5]
    );
    assert_eq!(
        retention_candidates_with_ancestry(&history, 6, &[], 2).unwrap(),
        Vec::<u64>::new()
    );
}

#[test]
fn rejects_incomplete_histories_even_when_a_bad_edge_is_not_retained() {
    let valid = [
        GenerationParent {
            generation: 1,
            parent: None,
        },
        GenerationParent {
            generation: 2,
            parent: Some(1),
        },
    ];
    assert!(retention_candidates_with_ancestry(&valid, 3, &[], 1).is_err());
    assert!(retention_candidates_with_ancestry(&valid, 2, &[3], 1).is_err());
    let mut bad = valid;
    bad[1].parent = Some(3);
    assert!(retention_candidates_with_ancestry(&bad, 1, &[], 0).is_err());
    bad[1].parent = Some(2);
    assert!(retention_candidates_with_ancestry(&bad, 1, &[], 0).is_err());
    bad[1].parent = Some(0);
    assert!(retention_candidates_with_ancestry(&bad, 1, &[], 0).is_err());
    bad[1] = bad[0];
    assert!(retention_candidates_with_ancestry(&bad, 1, &[], 0).is_err());
    assert!(retention_candidates_with_ancestry(&[], 1, &[], 0).is_err());
}

#[test]
fn balanced_histories_match_an_independent_mark_and_sweep_oracle() {
    use std::collections::{BTreeMap, BTreeSet};
    for size in [2u64, 3, 17, 128, 1024] {
        let mut history = (1..=size)
            .map(|generation| GenerationParent {
                generation,
                parent: (generation > 1).then_some(generation / 2),
            })
            .collect::<Vec<_>>();
        history.reverse();
        let parents = history
            .iter()
            .map(|entry| (entry.generation, entry.parent))
            .collect::<BTreeMap<_, _>>();
        for keep_newest in [0, 1, 2, 7] {
            let active = size / 2 + 1;
            let pinned = [size / 3 + 1];
            let mut retained = BTreeSet::new();
            let mut anchors = vec![active, pinned[0]];
            anchors.extend((1..=size).rev().take(keep_newest));
            for anchor in anchors {
                let mut node = Some(anchor);
                while let Some(generation) = node {
                    retained.insert(generation);
                    node = parents[&generation];
                }
            }
            let expected = (1..=size)
                .filter(|generation| !retained.contains(generation))
                .collect::<Vec<_>>();
            assert_eq!(
                retention_candidates_with_ancestry(&history, active, &pinned, keep_newest)
                    .unwrap(),
                expected
            );
        }
    }
}

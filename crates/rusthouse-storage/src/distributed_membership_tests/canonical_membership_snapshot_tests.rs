use super::*;

#[test]
fn equal_memberships_produce_identical_sorted_bytes() {
    let ordered = parse_membership("zeta west 3\nalpha east 1\ngamma east 2\n").unwrap();
    let reversed = ordered.iter().rev().cloned().collect::<Vec<_>>();
    let a = canonical_membership_manifest(&ordered).unwrap();
    let b = canonical_membership_manifest(&reversed).unwrap();
    assert_eq!(a, b);
    assert_eq!(a, "alpha east 1\ngamma east 2\nzeta west 3\n");
    assert_eq!(parse_membership(&a).unwrap().len(), 3);
    assert_ne!(format_membership(&ordered).unwrap(), a);
    assert_eq!(
        format_membership(&reversed).unwrap(),
        "gamma east 2\nalpha east 1\nzeta west 3\n"
    );
}

#[test]
fn large_snapshot_still_obeys_exact_wire_limit_and_uniqueness() {
    let nodes = (0..64)
        .rev()
        .map(|index| Member {
            node: format!("node_{index:02}"),
            zone: format!("zone_{}", index % 4),
            weight: index as u16 + 1,
        })
        .collect::<Vec<_>>();
    let canonical = canonical_membership_manifest(&nodes).unwrap();
    assert_eq!(parse_membership(&canonical).unwrap().len(), nodes.len());
    assert_eq!(
        canonical_membership_manifest(&parse_membership(&canonical).unwrap()).unwrap(),
        canonical
    );
    let mut duplicates = nodes;
    duplicates.push(duplicates[0].clone());
    assert!(canonical_membership_manifest(&duplicates).is_err());
}

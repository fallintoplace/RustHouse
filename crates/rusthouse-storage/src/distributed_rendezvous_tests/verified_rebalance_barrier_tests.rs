use super::*;
use crate::distributed_membership::parse_membership;

#[test]
fn old_replicas_wait_until_all_incoming_copies_are_verified() {
    let previous = parse_membership("a west 5\nb east 3\nc north 2\n").unwrap();
    let next = parse_membership("c north 2\nd south 8\ne east 1\n").unwrap();
    let movement = plan_zone_aware_rebalance("events", &previous, &next, 3).unwrap();
    assert_eq!(movement.incoming.len(), 2);
    assert_eq!(movement.outgoing.len(), 2);

    let initial =
        plan_guarded_zone_rebalance_step("events", &previous, &next, 3, &[], 1).unwrap();
    assert_eq!(initial.copy_to, movement.incoming[..1]);
    assert_eq!(initial.awaiting_verification, movement.incoming);
    assert!(initial.retire_after_verification.is_empty());

    let first = movement.incoming[0].as_str();
    let partial =
        plan_guarded_zone_rebalance_step("events", &previous, &next, 3, &[first], 1).unwrap();
    assert_eq!(partial.awaiting_verification, movement.incoming[1..]);
    assert_eq!(partial.copy_to, movement.incoming[1..]);
    assert!(partial.retire_after_verification.is_empty());

    let verified = movement
        .incoming
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let completed =
        plan_guarded_zone_rebalance_step("events", &previous, &next, 3, &verified, 0).unwrap();
    assert!(completed.copy_to.is_empty());
    assert!(completed.awaiting_verification.is_empty());
    assert_eq!(completed.retire_after_verification, movement.outgoing);
    assert_eq!(completed.new_placement, movement.new_placement);
}

#[test]
fn validates_verification_even_if_no_new_copy_can_be_scheduled() {
    let previous = parse_membership("a west 1\nb east 1\n").unwrap();
    let next = parse_membership("c west 1\nd east 1\n").unwrap();
    for verified in [vec!["unknown"], vec!["c", "c"]] {
        assert!(
            plan_guarded_zone_rebalance_step("events", &previous, &next, 2, &verified, 0,)
                .is_err()
        );
    }
    assert!(plan_guarded_zone_rebalance_step("events", &previous, &next, 2, &[], 65,).is_err());
}

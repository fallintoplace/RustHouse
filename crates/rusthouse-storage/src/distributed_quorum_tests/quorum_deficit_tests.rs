use super::*;

#[test]
fn counts_missing_votes_without_double_counting_duplicates() {
    let policy = QuorumPolicy::new(3, 2, 2).unwrap();
    let members = ["a", "b", "c"];
    assert_eq!(
        policy.deficits(&members, &["a", "a"]).unwrap(),
        QuorumDeficit {
            distinct_acks: 1,
            missing_reads: 1,
            missing_writes: 1
        }
    );
    assert_eq!(
        policy
            .deficits(&members, &["a", "b"])
            .unwrap()
            .missing_writes,
        0
    );
    assert!(policy.deficits(&members, &["unknown"]).is_err());
}

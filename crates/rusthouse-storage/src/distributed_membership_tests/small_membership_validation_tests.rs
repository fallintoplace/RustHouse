use super::*;

#[test]
fn small_and_large_validation_reject_the_same_duplicates() {
    for count in [1, 2, 3, 8, 9, 16, 64] {
        let valid = (0..count)
            .map(|index| Member {
                node: format!("node_{index}"),
                zone: format!("zone_{}", index % 3),
                weight: 1,
            })
            .collect::<Vec<_>>();
        assert!(validate_membership(&valid).is_ok());
        if count > 1 {
            let mut invalid = valid.clone();
            invalid[count - 1].node = invalid[0].node.clone();
            assert!(validate_membership(&invalid).is_err());
        }
    }
}

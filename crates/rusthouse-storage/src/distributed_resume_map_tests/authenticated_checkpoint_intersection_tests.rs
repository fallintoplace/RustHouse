use super::*;

#[test]
fn intersection_never_promotes_a_chunk_without_both_verifications() {
    for chunks in [0, 1, 7, 8, 9, 63, 64, 65, 511, 512, 65_535, 65_536] {
        let mut a = ResumeMap::new(chunks).unwrap();
        let mut b = ResumeMap::new(chunks).unwrap();
        for index in 0..chunks {
            if index % 3 != 0 {
                a.mark_complete(index).unwrap();
            }
            if index % 5 != 0 {
                b.mark_complete(index).unwrap();
            }
        }
        let mut intersection = a.clone();
        intersection.retain_common_verified(&b).unwrap();
        let expected = (0..chunks)
            .filter(|index| index % 3 != 0 && index % 5 != 0)
            .collect::<Vec<_>>();
        assert_eq!(intersection.completed_count(), expected.len());
        let actual = intersection
            .encode()
            .bytes()
            .enumerate()
            .filter_map(|(index, flag)| (flag == b'1').then_some(index))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        let mut reverse = b.clone();
        reverse.retain_common_verified(&a).unwrap();
        assert_eq!(reverse, intersection);
        intersection
            .retain_common_verified(&intersection.clone())
            .unwrap();
        assert_eq!(reverse, intersection);
    }
}

#[test]
fn mismatched_layout_rejection_does_not_change_destination() {
    let mut original = ResumeMap::new(9).unwrap();
    original.mark_complete(8).unwrap();
    let before = original.clone();
    assert!(
        original
            .retain_common_verified(&ResumeMap::new(8).unwrap())
            .is_err()
    );
    assert_eq!(original, before);
}

use super::*;

#[test]
fn word_popcounts_and_full_checks_match_bitwise_reference() {
    for bits in [0, 1, 7, 8, 9, 63, 64, 65, 511, 512, 513, 65_535, 65_536] {
        let mut progress = ResumeMap::new(bits).unwrap();
        for index in 0..bits {
            if index % 7 != 2 && index % 11 != 3 {
                progress.mark_complete(index).unwrap();
            }
        }
        let expected = (0..bits)
            .filter(|&index| index % 7 != 2 && index % 11 != 3)
            .count();
        assert_eq!(progress.completed_count(), expected, "bits={bits}");
        assert_eq!(progress.complete(), expected == bits, "bits={bits}");
        progress.mark_complete_range(0, bits).unwrap();
        assert_eq!(progress.completed_count(), bits);
        assert!(progress.complete());
    }
}

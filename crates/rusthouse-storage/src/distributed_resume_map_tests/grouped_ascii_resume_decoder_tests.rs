use super::*;

#[test]
fn decodes_bit_patterns_in_eight_character_groups() {
    for length in [0, 1, 7, 8, 9, 15, 16, 17, 511, 512, 513, 65_536] {
        let encoded = (0..length)
            .map(|index| if index % 5 < 2 { '1' } else { '0' })
            .collect::<String>();
        let map = ResumeMap::decode(&encoded).unwrap();
        assert_eq!(map.encode(), encoded);
        assert_eq!(
            map.completed_count(),
            (0..length).filter(|i| i % 5 < 2).count()
        );
    }
    for encoded in ["10x", "00000000x", "11111111\u{00a0}"] {
        assert!(ResumeMap::decode(encoded).is_err());
    }
}

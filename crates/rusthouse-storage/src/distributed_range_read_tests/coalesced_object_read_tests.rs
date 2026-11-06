use super::*;

#[test]
fn interval_union_matches_a_bytewise_reference_model() {
    for count in [0usize, 1, 2, 17, 128, 1024] {
        let ranges = (0..count)
            .map(|index| ((index * 13 % 201) as u64, index * 7 % 17))
            .collect::<Vec<_>>();
        let spans = plan_coalesced_object_ranges(&ranges, 256, 8 * 1024 * 1024).unwrap();
        assert!(
            spans
                .windows(2)
                .all(|pair| { pair[0].offset + (pair[0].length as u64) < pair[1].offset })
        );
        let mut expected = [false; 256];
        let mut actual = [false; 256];
        for &(offset, length) in &ranges {
            expected[offset as usize..offset as usize + length].fill(true);
        }
        for span in spans {
            actual[span.offset as usize..span.offset as usize + span.length].fill(true);
        }
        assert_eq!(actual, expected, "count={count}");
    }
}

#[test]
fn coalesced_reads_match_independent_exact_ranges_for_overlapping_inputs() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-coalesced-reads-{}-{}",
        std::process::id(),
        line!()
    ));
    let contents = (0..4096)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(&path, &contents).unwrap();
    let mut first = [0u8; 49];
    let mut second = [0u8; 40];
    let mut third = [0u8; 11];
    let mut empty = [];
    {
        let mut requests = [
            (95u64, first.as_mut_slice()),
            (101, second.as_mut_slice()),
            (135, third.as_mut_slice()),
            (4096, empty.as_mut_slice()),
        ];
        read_coalesced_object_ranges_into(&path, &mut requests, 100).unwrap();
    }
    assert_eq!(&first, &contents[95..144]);
    assert_eq!(&second, &contents[101..141]);
    assert_eq!(&third, &contents[135..146]);
    fs::remove_file(path).unwrap();
}

#[test]
fn invalid_ranges_fail_without_touching_destinations() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-invalid-coalesced-{}-{}",
        std::process::id(),
        line!()
    ));
    fs::write(&path, [7u8; 32]).unwrap();
    let mut good = [0xabu8; 4];
    let mut bad = [0xcdu8; 8];
    {
        let mut requests = [(0u64, good.as_mut_slice()), (30, bad.as_mut_slice())];
        assert!(read_coalesced_object_ranges_into(&path, &mut requests, 12).is_err());
    }
    assert_eq!(good, [0xab; 4]);
    assert_eq!(bad, [0xcd; 8]);
    assert!(plan_coalesced_object_ranges(&[(u64::MAX, 2)], u64::MAX, 10).is_err());
    assert!(plan_coalesced_object_ranges(&[(0, 5), (0, 5)], 32, 9).is_err());
    fs::remove_file(path).unwrap();
}

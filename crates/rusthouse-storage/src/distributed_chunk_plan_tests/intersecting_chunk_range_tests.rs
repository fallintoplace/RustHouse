use super::*;

#[test]
fn matches_bruteforce_byte_intersection_for_every_small_layout() {
    for total in 0..=32u64 {
        for width in 1..=12u64 {
            let planned = plan_chunks(total, width, 65_536).unwrap();
            for offset in 0..=total {
                for length in 0..=total - offset {
                    let indexes =
                        chunks_intersecting_range(total, width, 65_536, offset, length)
                            .unwrap();
                    let expected = planned
                        .iter()
                        .enumerate()
                        .filter(|(_, chunk)| {
                            length != 0
                                && chunk.offset < offset + length
                                && offset < chunk.offset + chunk.length
                        })
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>();
                    assert_eq!(
                        indexes.clone().collect::<Vec<_>>(),
                        expected,
                        "total={total}, width={width}, offset={offset}, length={length}"
                    );
                }
            }
        }
    }
}

#[test]
fn handles_last_partial_chunk_and_checked_integer_boundaries() {
    assert_eq!(chunks_intersecting_range(10, 4, 3, 7, 3).unwrap(), 1..3);
    assert_eq!(chunks_intersecting_range(10, 4, 3, 10, 0).unwrap(), 2..2);
    assert_eq!(chunks_intersecting_range(0, 4, 3, 0, 0).unwrap(), 0..0);
    assert_eq!(
        chunks_intersecting_range(u64::MAX, u64::MAX, 1, u64::MAX - 1, 1).unwrap(),
        0..1
    );
    assert!(chunks_intersecting_range(10, 4, 3, 10, 1).is_err());
    assert!(chunks_intersecting_range(10, 4, 3, 9, u64::MAX).is_err());
    assert!(chunks_intersecting_range(10, 0, 3, 0, 0).is_err());
    assert!(chunks_intersecting_range(10, 4, 2, 0, 0).is_err());
}

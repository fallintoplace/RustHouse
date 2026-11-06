use super::*;

#[test]
fn strict_and_clamped_windows_match_within_bounds() {
    for total in [0_u64, 1, 7, 8, 9, 33, 127, 256, 1025] {
        let count = total.div_ceil(7);
        for start in 0..=count {
            for request in 0..=count - start {
                let strict =
                    plan_chunk_window(total, 7, 65536, start as usize, request as usize)
                        .unwrap();
                let clamped = plan_chunk_window_clamped(
                    total,
                    7,
                    65536,
                    start as usize,
                    request as usize,
                )
                .unwrap();
                assert_eq!(strict, clamped);
            }
        }
    }
}

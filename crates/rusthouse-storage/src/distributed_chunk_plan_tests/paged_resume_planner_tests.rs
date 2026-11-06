use super::*;
use crate::distributed_resume_map::ResumeMap;

#[test]
fn cursored_pages_match_original_planner_on_filtered_missing_indexes() {
    for bytes in [0u64, 1, 5, 8, 9, 63, 64, 4097] {
        let chunks = bytes.div_ceil(5) as usize;
        let mut map = ResumeMap::new(chunks).unwrap();
        for index in (0..chunks).step_by(3) {
            map.mark_complete(index).unwrap();
        }
        for start in [0, 1, 7, 8, chunks / 2, chunks] {
            if start > chunks {
                continue;
            }
            let subset = map
                .remaining()
                .into_iter()
                .filter(|&index| index >= start)
                .collect::<Vec<_>>();
            for budget in [0, 1, 4, 5, 9, 100] {
                for limit in [0, 1, 2, 16, 128] {
                    let expected =
                        plan_missing_chunks(bytes, 5, 65_536, &subset, budget, limit).unwrap();
                    let actual = plan_missing_chunks_from_resume_after(
                        bytes, 5, 65_536, &map, start, budget, limit,
                    )
                    .unwrap();
                    assert_eq!(actual, expected);
                }
            }
        }
        assert!(
            plan_missing_chunks_from_resume_after(bytes, 5, 65_536, &map, chunks + 1, 0, 0)
                .is_err()
        );
    }
}

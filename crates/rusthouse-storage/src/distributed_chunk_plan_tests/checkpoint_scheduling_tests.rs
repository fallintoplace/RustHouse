use super::*;
use crate::distributed_resume_map::ResumeMap;

#[test]
fn agrees_with_indexed_planner_across_sparse_and_dense_checkpoints() {
    for bytes in [0u64, 1, 7, 8, 9, 65, 4097, 65_536] {
        let count = bytes.div_ceil(7) as usize;
        for complete_modulus in [1, 2, 7, 31] {
            let mut map = ResumeMap::new(count).unwrap();
            for index in (0..count).step_by(complete_modulus) {
                map.mark_complete(index).unwrap();
            }
            for budget in [0, 1, 2, 6, 7, 13, 14, 21, 1000] {
                for limit in [0, 1, 2, 5, 128] {
                    let expected =
                        plan_missing_chunks(bytes, 7, 65_536, &map.remaining(), budget, limit)
                            .unwrap();
                    let actual =
                        plan_missing_chunks_from_resume(bytes, 7, 65_536, &map, budget, limit)
                            .unwrap();
                    assert_eq!(
                        actual, expected,
                        "{bytes} {complete_modulus} {budget} {limit}"
                    );
                }
            }
        }
    }
}

#[test]
fn transfer_bounds_and_mismatched_checkpoints_fail_closed() {
    let map = ResumeMap::new(2).unwrap();
    assert!(plan_missing_chunks_from_resume(10, 4, 3, &map, 0, 0).is_err());
    assert!(plan_missing_chunks_from_resume(10, 0, 3, &map, 10, 3).is_err());
    assert!(plan_missing_chunks_from_resume(8, 4, 3, &map, 8, 65_537).is_err());
    let huge = ResumeMap::new(65_536).unwrap();
    let selected = plan_missing_chunks_from_resume(65_536, 1, 65_536, &huge, 3, 3).unwrap();
    assert_eq!(selected.len(), 3);
    assert_eq!(selected[2].index, 2);
}

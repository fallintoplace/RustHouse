use super::*;

#[test]
fn tiny_budget_only_schedules_the_short_final_chunk() {
    let missing = (0..65_536).collect::<Vec<_>>();
    let total_bytes = 65_535 * 8 + 3;
    let selected = plan_missing_chunks(total_bytes, 8, 65_536, &missing, 3, 4).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].index, 65_535);
    assert_eq!(selected[0].chunk.length, 3);
    assert!(
        plan_missing_chunks(total_bytes, 8, 65_536, &missing, 2, 4)
            .unwrap()
            .is_empty()
    );
    assert!(
        plan_missing_chunks(total_bytes, 8, 65_536, &missing[..65_535], 3, 4)
            .unwrap()
            .is_empty()
    );
}

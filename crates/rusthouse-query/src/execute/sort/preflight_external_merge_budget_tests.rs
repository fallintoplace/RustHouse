use super::*;

#[test]
fn predicts_bytes_of_merged_runs_without_writing_new_output() {
    let sorted = SortedRows::new(None, false);
    let temp = SortScratch::new().unwrap();
    let first = temp.0.join("source-1.bin");
    let second = temp.0.join("source-2.bin");
    fs::write(&first, vec![0u8; 1024]).unwrap();
    fs::write(&second, vec![0u8; 3072]).unwrap();
    let paths = vec![first, second];
    assert!(sorted.preflight_merge_run(&paths).is_ok());
    let mut nearly_full = sorted;
    nearly_full.temp_bytes = MAX_TEMP_SORT_BYTES - 2_048;
    assert!(nearly_full.preflight_merge_run(&paths).is_err());
}

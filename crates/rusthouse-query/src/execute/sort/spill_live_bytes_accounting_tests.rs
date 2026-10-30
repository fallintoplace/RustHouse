use super::*;

#[test]
fn deleting_superseded_runs_returns_their_disk_budget() {
    let mut sorted = SortedRows::new(None, false);
    let run = sorted.next_path().unwrap();
    fs::write(&run, vec![7u8; 4096]).unwrap();
    sorted.charge_temp(&run).unwrap();
    assert_eq!(sorted.temp_bytes, 4096);
    sorted.delete_run(&run).unwrap();
    assert_eq!(sorted.temp_bytes, 0);
    assert!(!run.exists());
}

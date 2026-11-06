use super::*;
use std::time::{Duration, SystemTime};

#[test]
fn a_same_size_overwrite_is_not_a_stable_immutable_object() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-range-open-metadata-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::write(&path, b"original").unwrap();
    let before = fs::symlink_metadata(&path).unwrap();
    // Force a distinct timestamp even on filesystems with coarse clocks.
    let file = File::options().write(true).open(&path).unwrap();
    file.set_times(
        std::fs::FileTimes::new()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000)),
    )
    .unwrap();
    let after = file.metadata().unwrap();
    assert_eq!(before.len(), after.len());
    assert!(!object_state_unchanged(&before, &after));
    fs::remove_file(path).unwrap();
}

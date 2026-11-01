#![cfg(unix)]
use std::fs;
use std::os::fd::AsRawFd;
use std::process::Command;

unsafe extern "C" {
    fn flock(fd: std::os::raw::c_int, operation: std::os::raw::c_int) -> std::os::raw::c_int;
}
const LOCK_EX: std::os::raw::c_int = 2;
const LOCK_NB: std::os::raw::c_int = 4;

#[test]
fn catalog_ddl_fails_closed_while_another_writer_owns_the_lock() {
    let catalog =
        std::env::temp_dir().join(format!("rusthouse-catalog-lock-{}", std::process::id()));
    let _ = fs::remove_dir_all(&catalog);
    fs::create_dir_all(&catalog).unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(catalog.join(".rusthouse-write.lock"))
        .unwrap();
    assert_eq!(unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) }, 0);
    let blocked = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "CREATE TABLE events (id UInt64)", "--catalog"])
        .arg(&catalog)
        .output()
        .unwrap();
    assert!(!blocked.status.success(), "{blocked:?}");
    assert!(!catalog.join("events.schema").exists());
    drop(file);
    let created = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "CREATE TABLE events (id UInt64)", "--catalog"])
        .arg(&catalog)
        .output()
        .unwrap();
    assert!(created.status.success(), "{created:?}");
    fs::remove_dir_all(catalog).unwrap();
}

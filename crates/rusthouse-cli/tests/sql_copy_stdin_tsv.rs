use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn run(db: &Path, statement: &str, input: Option<&[u8]>) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusthouse"));
    command
        .args(["sql", statement, "--database"])
        .arg(db)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    child.wait_with_output().unwrap()
}

#[test]
fn accepts_streamed_tsv_without_a_temporary_file() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-copy-pipe-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let created = run(&dir, "CREATE TABLE events (id UInt64, label String)", None);
    assert!(created.status.success(), "{created:?}");
    let copied = run(
        &dir,
        "COPY events FROM '-' WITH (FORMAT TSV)",
        Some(b"1\tfirst\n2\tsecond\n"),
    );
    assert!(copied.status.success(), "{copied:?}");
    let count = run(&dir, "SELECT count(*) FROM events", None);
    assert!(count.status.success(), "{count:?}");
    assert_eq!(count.stdout, b"count\n2\n");
    let rejected = run(
        &dir,
        "COPY events FROM '-' WITH (FORMAT CSV)",
        Some(b"3,third\n"),
    );
    assert!(!rejected.status.success());
    std::fs::remove_dir_all(dir).unwrap();
}

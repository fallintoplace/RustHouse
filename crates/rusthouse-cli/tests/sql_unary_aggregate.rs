use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn evaluates_signed_aggregate_arguments() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-unary-aggregate-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&db).unwrap();
    let run = |q: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", q, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for q in [
        "CREATE TABLE amounts (amount Int64)",
        "INSERT INTO amounts VALUES (5), (7)",
    ] {
        let result = run(q);
        assert!(result.status.success(), "{q}: {result:?}");
    }
    let result = run("SELECT SUM(-amount) AS negative, COUNT(*) AS total FROM amounts");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"negative\ttotal\n-12\t2\n");
    fs::remove_dir_all(db).unwrap();
}

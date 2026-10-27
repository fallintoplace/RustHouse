use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-query-matrix-mutations-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn run(db: &Path, statement: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn queries_remain_consistent_after_persisted_rewrites() {
    let db = database();
    for statement in [
        "CREATE TABLE inventory (id UInt64, status String, qty UInt64, note Nullable(String))",
        "INSERT INTO inventory VALUES (1, 'new', 2, NULL), (2, 'held', 5, 'a'), (3, 'new', 7, 'x'), (4, 'done', 0, NULL), (5, 'held', 3, 'b')",
    ] {
        run(&db, statement);
    }
    assert_eq!(
        run(&db, "SELECT id, status FROM inventory WHERE qty > 1 ORDER BY id"),
        "id\tstatus\n1\tnew\n2\theld\n3\tnew\n5\theld\n"
    );
    run(&db, "UPDATE inventory SET status = 'done' WHERE status = 'new'");
    assert_eq!(
        run(&db, "SELECT status, count(*) AS n FROM inventory GROUP BY status ORDER BY status"),
        "status\tn\ndone\t3\nheld\t2\n"
    );
    run(&db, "DELETE FROM inventory WHERE qty = 0");
    assert_eq!(
        run(&db, "SELECT id, status FROM inventory ORDER BY id DESC LIMIT 2"),
        "id\tstatus\n5\theld\n3\tdone\n"
    );
    run(&db, "INSERT INTO inventory VALUES (6, 'done', 9, NULL)");
    assert_eq!(
        run(&db, "SELECT count(*) AS remaining, count(note) AS tagged FROM inventory WHERE status = 'done'"),
        "remaining\ttagged\n3\t1\n"
    );
    fs::remove_dir_all(db).unwrap();
}

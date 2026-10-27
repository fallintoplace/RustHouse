use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-query-matrix-joins-{}-{}",
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
fn joins_preserve_duplicates_unmatched_rows_and_pagination() {
    let db = database();
    for statement in [
        "CREATE TABLE employees (id UInt64, team Nullable(UInt64), name String)",
        "CREATE TABLE teams (id Nullable(UInt64), label String)",
        "INSERT INTO employees VALUES (1, 10, 'Ada'), (2, 20, 'Bob'), (3, NULL, 'Cy'), (4, 10, 'Dee'), (5, 40, 'Eli')",
        "INSERT INTO teams VALUES (10, 'Blue'), (10, 'Cyan'), (20, 'Red'), (30, 'Gold'), (NULL, 'Void')",
    ] {
        run(&db, statement);
    }
    for (statement, expected) in [
        (
            "SELECT e.id, t.label FROM employees e JOIN teams t ON e.team = t.id ORDER BY e.id, t.label",
            "e.id\tt.label\n1\tBlue\n1\tCyan\n2\tRed\n4\tBlue\n4\tCyan\n",
        ),
        (
            "SELECT e.id FROM employees e LEFT JOIN teams t ON e.team = t.id WHERE t.label IS NULL ORDER BY e.id",
            "e.id\n3\n5\n",
        ),
        (
            "SELECT t.label, e.id FROM employees e RIGHT JOIN teams t ON e.team = t.id ORDER BY t.label DESC, e.id ASC",
            "t.label\te.id\nVoid\t\\N\nRed\t2\nGold\t\\N\nCyan\t1\nCyan\t4\nBlue\t1\nBlue\t4\n",
        ),
        (
            "SELECT e.id, t.label FROM employees e FULL JOIN teams t ON e.team = t.id ORDER BY e.id ASC NULLS LAST, t.label ASC LIMIT 4 OFFSET 4",
            "e.id\tt.label\n4\tBlue\n4\tCyan\n5\t\\N\n\\N\tGold\n",
        ),
    ] {
        assert_eq!(run(&db, statement), expected, "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}

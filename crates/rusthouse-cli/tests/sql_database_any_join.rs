use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn any_join_keeps_one_right_match_per_left_row() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-any-join-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE employees (id UInt64, team UInt64)",
        "CREATE TABLE teams (id UInt64, label String)",
        "INSERT INTO employees VALUES (1, 10), (2, 20), (3, 30)",
        "INSERT INTO teams VALUES (10, 'first'), (10, 'second'), (20, 'only')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }

    let inner =
        sql("SELECT e.id, t.label FROM employees e ANY INNER JOIN teams t ON e.team = t.id");
    assert!(inner.status.success(), "{inner:?}");
    assert_eq!(inner.stdout, b"e.id\tt.label\n1\tfirst\n2\tonly\n");

    let left = sql(
        "SELECT e.id, t.label FROM employees e LEFT ANY JOIN teams t ON e.team = t.id ORDER BY e.id LIMIT 10",
    );
    assert!(left.status.success(), "{left:?}");
    assert_eq!(left.stdout, b"e.id\tt.label\n1\tfirst\n2\tonly\n3\t\\N\n");

    let all = sql("SELECT e.id, t.label FROM employees e JOIN teams t ON e.team = t.id");
    assert!(all.status.success(), "{all:?}");
    assert_eq!(all.stdout, b"e.id\tt.label\n1\tfirst\n1\tsecond\n2\tonly\n");
    fs::remove_dir_all(db).unwrap();
}

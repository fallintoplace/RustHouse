use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(query: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn pages_and_returns_joined_pairs_across_old_65536_limit() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-large-page-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for command in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimension (id UInt64)",
    ] {
        let result = sql(command, &db);
        assert!(result.status.success(), "{command}: {result:?}");
    }
    for table in ["facts", "dimension"] {
        for _ in 0..3 {
            let command = format!("INSERT INTO {table} SELECT number FROM numbers(8192)");
            let result = sql(&command, &db);
            assert!(result.status.success(), "{command}: {result:?}");
        }
    }

    // Each of the 24,576 fact rows matches three dimension rows: 73,728 pairs.
    let page = sql(
        "SELECT f.id FROM facts f JOIN dimension d ON f.id = d.id LIMIT 2 OFFSET 65536",
        &db,
    );
    assert!(page.status.success(), "{page:?}");
    assert_eq!(page.stdout, b"f.id\n5461\n5461\n");

    // The 73,728 matched rows now fit without requiring a LIMIT.
    let unbounded = sql(
        "SELECT f.id FROM facts f JOIN dimension d ON f.id = d.id",
        &db,
    );
    assert!(unbounded.status.success(), "{unbounded:?}");
    assert_eq!(
        unbounded
            .stdout
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count(),
        73_729
    );

    // Another three copies of the dimension create 147,456 matched pairs.
    for _ in 0..3 {
        let appended = sql(
            "INSERT INTO dimension SELECT number FROM numbers(8192)",
            &db,
        );
        assert!(appended.status.success(), "{appended:?}");
    }
    let rejected = sql(
        "SELECT f.id FROM facts f JOIN dimension d ON f.id = d.id",
        &db,
    );
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("131072"));
    fs::remove_dir_all(db).unwrap();
}

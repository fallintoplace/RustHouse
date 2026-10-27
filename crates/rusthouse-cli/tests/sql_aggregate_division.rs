use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn computes_division_and_rejects_zero_divisors() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-aggregate-division-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&db).unwrap();
    let run = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for query in [
        "CREATE TABLE facts (price Float64, units Float64)",
        "INSERT INTO facts VALUES (10, 2), (9, 3)",
    ] {
        let result = run(query);
        assert!(result.status.success(), "{query}: {result:?}");
    }

    let result = run("SELECT SUM(price / units) AS ratio, COUNT(*) AS n FROM facts");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"ratio\tn\n8\t2\n");

    let inserted = run("INSERT INTO facts VALUES (1, 0)");
    assert!(inserted.status.success(), "{inserted:?}");
    let result = run("SELECT SUM(price / units) AS ratio, COUNT(*) AS n FROM facts");
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("division by zero"));
    fs::remove_dir_all(db).unwrap();
}

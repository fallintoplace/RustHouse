use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-grouped-topk-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
fn ok(db: &Path, statement: &str) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
#[test]
fn group_heavy_hitters_survive_long_tail_on_each_cohort() {
    let db = database();
    ok(&db, "CREATE TABLE events (region String, uid UInt64)");
    let input = db.join("events.tsv");
    let mut data = String::from("region\tuid\n");
    for user in 0..10_000 {
        use std::fmt::Write as _;
        writeln!(&mut data, "east\t{}", user + 100_000).unwrap();
        writeln!(&mut data, "west\t{}", user + 200_000).unwrap();
    }
    for (region, user, count) in [
        ("east", 1, 2500),
        ("east", 2, 1800),
        ("west", 4, 2500),
        ("west", 5, 1800),
    ] {
        for _ in 0..count {
            use std::fmt::Write as _;
            writeln!(&mut data, "{region}\t{user}").unwrap();
        }
    }
    fs::write(&input, data).unwrap();
    ok(&db, &format!("COPY events FROM '{}' WITH (FORMAT TSV, HEADER TRUE)", input.display()));
    assert_eq!(
        ok(
            &db,
            "SELECT region, topK(2)(uid) FROM events GROUP BY region ORDER BY region",
        ),
        "region\ttopK(2)(uid)\neast\t[1,2]\nwest\t[4,5]\n"
    );
    fs::remove_dir_all(db).unwrap();
}
#[test]
fn null_values_excluded_from_grouped_frequency_arrays() {
    let db = database();
    ok(&db, "CREATE TABLE events (region String, tag Nullable(String))");
    ok(&db, "INSERT INTO events VALUES ('east', 'blue'), ('east', NULL), ('east', 'blue'), ('west', NULL)");
    assert_eq!(
        ok(&db, "SELECT region, topK(2)(tag) FROM events GROUP BY region ORDER BY region"),
        "region\ttopK(2)(tag)\neast\t['blue']\nwest\t[]\n"
    );
    fs::remove_dir_all(db).unwrap();
}

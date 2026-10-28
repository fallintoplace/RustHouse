use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-composite-uniq-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
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
fn composite_uniqueness_counts_across_150k_fact_rows() {
    let db = database();
    ok(&db, "CREATE TABLE events (uid UInt64, region String)");
    let input = db.join("events.tsv");
    let mut content = String::from("uid\tregion\n");
    for id in 0..50_000u64 {
        use std::fmt::Write as _;
        writeln!(&mut content, "{id}\tnorth").unwrap();
        writeln!(&mut content, "{id}\tsouth").unwrap();
        writeln!(&mut content, "{id}\tsouth").unwrap();
    }
    fs::write(&input, content).unwrap();
    ok(&db, &format!("COPY events FROM '{}' WITH (FORMAT TSV, HEADER TRUE)", input.display()));
    let parse_estimate = |sql: &str| {
        ok(&db, sql).lines().nth(1).unwrap().parse::<u64>().unwrap()
    };
    let one = parse_estimate("SELECT uniq(uid) FROM events");
    let two = parse_estimate("SELECT uniq(uid,region) FROM events");
    assert!(one.abs_diff(50_000) < 2_500, "{one}");
    assert!(two.abs_diff(100_000) < 5_000, "{two}");
    let filtered = parse_estimate("SELECT uniq(uid,region) FROM events WHERE region = 'north'");
    assert!(filtered.abs_diff(50_000) < 2_500, "{filtered}");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn composite_nullable_values_skip_missing_members() {
    let db = database();
    ok(&db, "CREATE TABLE events (uid UInt64, tag Nullable(String))");
    ok(&db, "INSERT INTO events VALUES (1, 'blue'), (1, 'blue'), (1, NULL), (1, 'red'), (2, NULL)");
    assert_eq!(ok(&db, "SELECT uniq(uid,tag) FROM events"), "uniq(uid,tag)\n2\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn billion_generated_composite_uniq_uses_exact_cardinality_fast_path() {
    let db = database();
    assert_eq!(
        ok(&db, "SELECT uniq(number,number) FROM numbers(1000000000)"),
        "uniq(number,number)\n1000000000\n"
    );
    fs::remove_dir_all(db).unwrap();
}

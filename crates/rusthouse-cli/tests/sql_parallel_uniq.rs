use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-parallel-hll-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn ok(db: &Path, sql: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", sql, "--database"])
        .arg(db)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{sql}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn estimate(text: String) -> u64 {
    text.lines().nth(1).unwrap().parse().unwrap()
}

#[test]
fn parallel_multpart_uniq_is_globally_deduplicated_and_filterable() {
    let db = database();
    ok(&db, "CREATE TABLE events (uid UInt64)");
    // The immutable writer produces multiple independent physical parts.
    // Overlapping values across appends must be deduplicated globally.
    ok(&db, "INSERT INTO events SELECT number FROM numbers(280000)");
    ok(&db, "INSERT INTO events SELECT number FROM numbers(100000)");
    assert_eq!(ok(&db, "SELECT count(*) FROM events"), "count\n380000\n");
    let first = estimate(ok(&db, "SELECT uniq(uid) FROM events"));
    let second = estimate(ok(&db, "SELECT uniq(uid) FROM events"));
    assert_eq!(first, second);
    assert!(first.abs_diff(280000) < 14_000, "estimate={first}");
    let filtered = estimate(ok(&db, "SELECT uniq(uid) FROM events WHERE uid >= 100000"));
    assert!(filtered.abs_diff(180000) < 9_000, "estimate={filtered}");
    fs::remove_dir_all(db).unwrap();
}

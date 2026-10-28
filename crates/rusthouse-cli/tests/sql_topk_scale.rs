use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-topk-{}-{}",
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
fn billion_row_generated_unique_keys_use_direct_constant_time_page() {
    let db = database();
    assert_eq!(
        ok(&db, "SELECT topK(5)(number) FROM numbers(1000000000)"),
        "topK(5)(number)\n[0,1,2,3,4]\n"
    );
    assert_eq!(
        ok(&db, "SELECT topK(3)(number) FROM numbers(1000000000) WHERE number >= 999999000"),
        "topK(3)(number)\n[999999000,999999001,999999002]\n"
    );
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn frequent_events_survive_long_tail_and_respect_filters() {
    let db = database();
    ok(&db, "CREATE TABLE events (cohort String, uid UInt64)");
    let tsv = db.join("facts.tsv");
    let mut content = String::from("cohort\tuid\n");
    for id in 0..6000u64 {
        use std::fmt::Write as _;
        writeln!(&mut content, "west\t{}", id + 10000).unwrap();
    }
    for (id, count) in [(1, 7000), (2, 5000), (3, 3000)] {
        for _ in 0..count {
            use std::fmt::Write as _;
            writeln!(&mut content, "west\t{id}").unwrap();
        }
    }
    fs::write(&tsv, content).unwrap();
    ok(
        &db,
        &format!("COPY events FROM '{}' WITH (FORMAT TSV, HEADER TRUE)", tsv.display()),
    );
    assert_eq!(
        ok(&db, "SELECT topK(3)(uid) FROM events WHERE cohort = 'west'"),
        "topK(3)(uid)\n[1,2,3]\n"
    );
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn string_keys_and_nullable_values_are_supported() {
    let db = database();
    ok(&db, "CREATE TABLE events (tag Nullable(String))");
    ok(&db, "INSERT INTO events VALUES ('blue'), ('red'), ('blue'), (NULL), ('blue'), ('red')");
    assert_eq!(
        ok(&db, "SELECT topK(2)(tag) FROM events"),
        "topK(2)(tag)\n['blue','red']\n"
    );
    fs::remove_dir_all(db).unwrap();
}

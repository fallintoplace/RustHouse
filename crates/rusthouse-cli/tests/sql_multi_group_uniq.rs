use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-multi-group-uniq-{}-{}",
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
fn cohort_unique_users_across_two_dimensions() {
    let db = database();
    ok(&db, "CREATE TABLE events (region String, device UInt8, uid UInt64)");
    let tsv = db.join("facts.tsv");
    let mut data = String::from("region\tdevice\tuid\n");
    for uid in 0..30_000u64 {
        for region in ["east", "west"] {
            for device in 0..2 {
                use std::fmt::Write as _;
                writeln!(&mut data, "{region}\t{device}\t{uid}").unwrap();
            }
        }
    }
    fs::write(&tsv, data).unwrap();
    ok(&db, &format!("COPY events FROM '{}' WITH (FORMAT TSV, HEADER TRUE)", tsv.display()));
    let output = ok(
        &db,
        "SELECT region, device, uniq(uid) FROM events GROUP BY region, device HAVING uniq(uid) > 20000 ORDER BY 3 DESC, 1",
    );
    let rows = output.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 5, "{output}");
    assert_eq!(rows[0], "region\tdevice\tuniq(uid)");
    let mut previous = u64::MAX;
    for line in rows.iter().skip(1) {
        let estimate = line.split('\t').nth(2).unwrap().parse::<u64>().unwrap();
        assert!(estimate.abs_diff(30_000) < 1_500, "{estimate}");
        assert!(estimate <= previous);
        previous = estimate;
    }
    fs::remove_dir_all(db).unwrap();
}
#[test]
fn filtered_nullable_group_keys_keep_distinct_null_cohorts() {
    let db = database();
    ok(&db, "CREATE TABLE events (region Nullable(String), device UInt8, uid Nullable(UInt64))");
    ok(&db, "INSERT INTO events VALUES (NULL, 1, 5), (NULL, 1, NULL), ('east', 1, 5), ('east', 1, 6)");
    let output = ok(
        &db,
        "SELECT region, device, uniq(uid) FROM events GROUP BY region, device ORDER BY 1",
    );
    assert!(output.contains("NULL\t1\t1"), "{output}");
    assert!(output.contains("east\t1\t2"), "{output}");
    fs::remove_dir_all(db).unwrap();
}

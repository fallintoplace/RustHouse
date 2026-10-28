use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-grouped-uniq-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn ok(db: &Path, statement: &str) -> String {
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
fn large_fact_table_grouped_unique_users_with_order_and_having() {
    let db = database();
    ok(&db, "CREATE TABLE events (region UInt8, user_id UInt64)");
    let tsv = db.join("events.tsv");
    let mut contents = String::from("region\tuser_id\n");
    for user in 0..15_000u64 {
        for region in 0..8u8 {
            use std::fmt::Write as _;
            writeln!(&mut contents, "{region}\t{user}").unwrap();
        }
    }
    fs::write(&tsv, contents).unwrap();
    ok(
        &db,
        &format!(
            "COPY events FROM '{}' WITH (FORMAT TSV, HEADER TRUE)",
            tsv.display()
        ),
    );
    let output = ok(
        &db,
        "SELECT region, uniq(user_id) FROM events GROUP BY region ORDER BY 2 DESC LIMIT 4",
    );
    let lines = output.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "region\tuniq(user_id)");
    let mut previous = u64::MAX;
    for line in lines.iter().skip(1) {
        let count = line.split('\t').nth(1).unwrap().parse::<u64>().unwrap();
        assert!(count.abs_diff(15000) < 750, "{count}");
        assert!(count <= previous, "descending ORDER BY must sort estimates");
        previous = count;
    }
    let output = ok(
        &db,
        "SELECT region, uniq(user_id) FROM events GROUP BY region HAVING uniq(user_id) > 10000 ORDER BY region",
    );
    assert_eq!(output.lines().count(), 9);
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn null_values_do_not_contribute_to_each_group() {
    let db = database();
    ok(&db, "CREATE TABLE events (region String, user_id Nullable(UInt64))");
    ok(&db, "INSERT INTO events VALUES ('west', 11), ('west', NULL), ('west', 11), ('east', NULL)");
    assert_eq!(
        ok(&db, "SELECT region, uniq(user_id) FROM events GROUP BY region ORDER BY region"),
        "region\tuniq(user_id)\neast\t0\nwest\t1\n"
    );
    fs::remove_dir_all(db).unwrap();
}

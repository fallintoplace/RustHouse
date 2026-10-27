use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-copy-csv-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn sql(statement: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

fn ok(statement: &str, db: &Path) -> String {
    let result = sql(statement, db);
    assert!(
        result.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn copies_csv_with_named_headers_and_escaped_fields() {
    let db = database();
    ok(
        "CREATE TABLE events (id UInt64, name String, note Nullable(String))",
        &db,
    );
    let input = db.join("input.csv");
    fs::write(
        &input,
        "name,id,note\n\"Ada, Lovelace\",1,\n\"Grace\nHopper\",2,\"\"\n",
    )
    .unwrap();
    let statement = format!(
        "COPY events FROM '{}' WITH (FORMAT CSV, HEADER TRUE)",
        input.display()
    );
    ok(&statement, &db);
    assert_eq!(
        ok("SELECT id, name, note FROM events ORDER BY id", &db),
        "id\tname\tnote\n1\tAda, Lovelace\t\\N\n2\tGrace\\nHopper\t\n"
    );
    fs::write(&input, "id,name,note\n3,\"bad\n").unwrap();
    assert!(!sql(&statement, &db).status.success());
    assert_eq!(ok("SELECT count(*) FROM events", &db), "count\n2\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn rejects_header_mismatch_and_preserves_existing_rows() {
    let db = database();
    ok("CREATE TABLE events (id UInt64, name String)", &db);
    let input = db.join("input.csv");
    fs::write(&input, "id,name\n1,Ada\n").unwrap();
    let wrong = format!(
        "COPY events (name, id) FROM '{}' WITH (FORMAT CSV, HEADER TRUE)",
        input.display()
    );
    assert!(!sql(&wrong, &db).status.success());
    assert_eq!(ok("SELECT count(*) FROM events", &db), "count\n0\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn bulk_copy_publishes_all_parts_or_no_parts() {
    use std::fmt::Write as _;

    let db = database();
    ok("CREATE TABLE events (id UInt64, name String)", &db);
    ok("INSERT INTO events VALUES (9000, 'existing')", &db);
    let input = db.join("bulk.csv");
    let mut csv = String::from("id,name\n");
    for id in 0..5100 {
        writeln!(&mut csv, "{id},item-{id}").unwrap();
    }
    fs::write(&input, &csv).unwrap();
    let statement = format!(
        "COPY events FROM '{}' WITH (FORMAT CSV, HEADER TRUE)",
        input.display()
    );
    ok(&statement, &db);
    assert_eq!(ok("SELECT count(*) FROM events", &db), "count\n5101\n");
    let pointer = db.join("events.parts/.rusthouse-active");
    assert_eq!(fs::read_to_string(&pointer).unwrap(), "g1\n");

    // An invalid value must not alter the already-active generation.
    csv.push_str("not-an-integer,invalid\n");
    fs::write(&input, &csv).unwrap();
    let rejected = sql(&statement, &db);
    assert!(!rejected.status.success(), "{rejected:?}");
    assert_eq!(fs::read_to_string(&pointer).unwrap(), "g1\n");
    assert_eq!(ok("SELECT count(*) FROM events", &db), "count\n5101\n");
    fs::remove_dir_all(db).unwrap();
}

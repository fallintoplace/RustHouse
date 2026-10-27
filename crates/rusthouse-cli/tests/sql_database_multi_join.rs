use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-chain-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(query: &str, path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(path)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn chained_inner_join_preserves_duplicates_projection_order_and_paging() {
    let db = database();
    for statement in [
        "CREATE TABLE orders (id UInt64, customer_id Nullable(UInt64))",
        "CREATE TABLE customers (id UInt64, region_id UInt64, name String)",
        "CREATE TABLE regions (id UInt64, label String)",
        "INSERT INTO orders VALUES (1, 10), (2, 10), (3, 20), (4, NULL)",
        "INSERT INTO customers VALUES (10, 1, 'Ada'), (10, 2, 'Amy'), (20, 3, 'Bob')",
        "INSERT INTO regions VALUES (1, 'Europe'), (2, 'Asia'), (3, 'America')",
    ] {
        let result = sql(statement, &db, &[]);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let joined = sql(
        "SELECT o.id, c.name, r.label FROM orders o JOIN customers c ON o.customer_id = c.id INNER JOIN regions r ON c.region_id = r.id ORDER BY o.id LIMIT 3 OFFSET 1",
        &db,
        &[],
    );
    assert!(joined.status.success(), "{joined:?}");
    assert_eq!(
        joined.stdout,
        b"o.id\tc.name\tr.label\n1\tAmy\tAsia\n2\tAda\tEurope\n2\tAmy\tAsia\n"
    );

    let wildcards = sql(
        "SELECT o.*, r.* FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r ON r.id = c.region_id ORDER BY o.id LIMIT 1",
        &db,
        &[],
    );
    assert!(wildcards.status.success(), "{wildcards:?}");
    assert_eq!(
        wildcards.stdout,
        b"o.id\to.customer_id\tr.id\tr.label\n1\t10\t1\tEurope\n"
    );

    let plan = sql(
        "SELECT * FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r ON c.region_id = r.id",
        &db,
        &["--explain"],
    );
    assert!(plan.status.success(), "{plan:?}");
    assert!(String::from_utf8_lossy(&plan.stdout).contains("table regions AS r"));
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn chained_inner_join_rejects_invalid_keys_without_writing_rows() {
    let db = database();
    for statement in [
        "CREATE TABLE orders (id UInt64)",
        "CREATE TABLE customers (id UInt64)",
        "CREATE TABLE regions (label String)",
        "INSERT INTO orders VALUES (1)",
        "INSERT INTO customers VALUES (1)",
        "INSERT INTO regions VALUES ('one')",
    ] {
        let result = sql(statement, &db, &[]);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let invalid = sql(
        "SELECT * FROM orders o JOIN customers c ON o.id = c.id JOIN regions r ON c.id = r.label",
        &db,
        &[],
    );
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("JOIN key types differ"));
    let unknown = sql(
        "SELECT * FROM orders o JOIN customers c ON o.id = c.id JOIN regions r ON x.id = r.label",
        &db,
        &[],
    );
    assert!(!unknown.status.success());
    assert!(unknown.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown JOIN qualifier"));
    fs::remove_dir_all(db).unwrap();
}

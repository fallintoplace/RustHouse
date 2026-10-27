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

#[test]
fn chained_left_joins_null_extend_each_unmatched_relation() {
    let db = database();
    for statement in [
        "CREATE TABLE orders (id UInt64, customer_id Nullable(UInt64))",
        "CREATE TABLE customers (id UInt64, region_id UInt64, name String)",
        "CREATE TABLE regions (id UInt64, label String)",
        "INSERT INTO orders VALUES (1, 10), (2, NULL), (3, 20)",
        "INSERT INTO customers VALUES (10, 5, 'Ada'), (20, 99, 'Bob')",
        "INSERT INTO regions VALUES (5, 'East')",
    ] {
        let result = sql(statement, &db, &[]);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let joined = sql(
        "SELECT o.id, c.name, r.label FROM orders o LEFT JOIN customers c ON o.customer_id = c.id LEFT JOIN regions r ON c.region_id = r.id ORDER BY o.id LIMIT 10",
        &db,
        &[],
    );
    assert!(joined.status.success(), "{joined:?}");
    assert_eq!(
        joined.stdout,
        b"o.id\tc.name\tr.label\n1\tAda\tEast\n2\t\\N\t\\N\n3\tBob\t\\N\n"
    );
    let missing = sql(
        "SELECT o.id FROM orders o LEFT JOIN customers c ON o.customer_id = c.id LEFT JOIN regions r ON c.region_id = r.id WHERE r.label IS NULL ORDER BY o.id",
        &db,
        &[],
    );
    assert!(missing.status.success(), "{missing:?}");
    assert_eq!(missing.stdout, b"o.id\n2\n3\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn chained_right_and_full_joins_preserve_unmatched_right_rows() {
    let db = database();
    for statement in [
        "CREATE TABLE orders (id UInt64, customer_id UInt64)",
        "CREATE TABLE customers (id UInt64, region_id UInt64)",
        "CREATE TABLE regions (id UInt64, label String)",
        "INSERT INTO orders VALUES (1, 10), (2, 20)",
        "INSERT INTO customers VALUES (10, 1), (20, 2)",
        "INSERT INTO regions VALUES (1, 'West'), (3, 'East')",
    ] {
        let result = sql(statement, &db, &[]);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let right = sql(
        "SELECT o.id, c.id, r.label FROM orders o JOIN customers c ON o.customer_id = c.id RIGHT JOIN regions r ON c.region_id = r.id ORDER BY r.id",
        &db,
        &[],
    );
    assert!(right.status.success(), "{right:?}");
    assert_eq!(right.stdout, b"o.id\tc.id\tr.label\n1\t10\tWest\n\\N\t\\N\tEast\n");

    let full = sql(
        "SELECT o.id, c.id, r.label FROM orders o JOIN customers c ON o.customer_id = c.id FULL JOIN regions r ON c.region_id = r.id ORDER BY r.id ASC NULLS LAST",
        &db,
        &[],
    );
    assert!(full.status.success(), "{full:?}");
    assert_eq!(
        full.stdout,
        b"o.id\tc.id\tr.label\n1\t10\tWest\n\\N\t\\N\tEast\n2\t20\t\\N\n"
    );
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn chained_any_joins_select_only_one_matching_row_at_each_step() {
    let db = database();
    for statement in [
        "CREATE TABLE orders (id UInt64, customer_id UInt64)",
        "CREATE TABLE customers (id UInt64, region_id UInt64)",
        "CREATE TABLE regions (id UInt64, label String)",
        "INSERT INTO orders VALUES (1, 10), (2, 20)",
        "INSERT INTO customers VALUES (10, 1), (10, 2)",
        "INSERT INTO regions VALUES (1, 'One'), (1, 'Uno'), (2, 'Two')",
    ] {
        let result = sql(statement, &db, &[]);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT o.id, c.region_id, r.label FROM orders o LEFT ANY JOIN customers c ON o.customer_id = c.id LEFT ANY JOIN regions r ON c.region_id = r.id ORDER BY o.id",
        &db,
        &[],
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"o.id\tc.region_id\tr.label\n1\t1\tOne\n2\t\\N\t\\N\n"
    );
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn chained_using_joins_coalesce_star_keys_for_full_outer_rows() {
    let db = database();
    for statement in [
        "CREATE TABLE orders (id UInt64, customer_id UInt64)",
        "CREATE TABLE customers (id UInt64, region_id UInt64, name String)",
        "CREATE TABLE regions (region_id UInt64, label String)",
        "INSERT INTO orders VALUES (1, 10), (2, 20)",
        "INSERT INTO customers VALUES (10, 1, 'Ada'), (20, 2, 'Bob')",
        "INSERT INTO regions VALUES (1, 'West'), (3, 'East')",
    ] {
        let result = sql(statement, &db, &[]);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT * FROM orders o JOIN customers c ON o.customer_id = c.id FULL JOIN regions r USING (region_id) ORDER BY r.region_id ASC NULLS LAST",
        &db,
        &[],
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"o.id\to.customer_id\tc.id\tc.region_id\tc.name\tr.label\n1\t10\t10\t1\tAda\tWest\n\\N\t\\N\t\\N\t3\t\\N\tEast\n2\t20\t20\t2\tBob\t\\N\n"
    );
    let plan = sql(
        "SELECT * FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r USING (region_id)",
        &db,
        &["--explain"],
    );
    assert!(plan.status.success(), "{plan:?}");
    assert!(String::from_utf8_lossy(&plan.stdout).contains("USING (region_id)"));

    let ambiguous = sql(
        "SELECT * FROM orders o JOIN customers c ON o.id = c.id JOIN regions r USING (id)",
        &db,
        &[],
    );
    assert!(!ambiguous.status.success());
    assert!(ambiguous.stdout.is_empty());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("ambiguous JOIN USING"));
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn chained_four_table_inner_join_can_reference_any_prior_alias() {
    let db = database();
    for statement in [
        "CREATE TABLE facts (id UInt64, account UInt64, region UInt64)",
        "CREATE TABLE accounts (id UInt64, name String)",
        "CREATE TABLE regions (id UInt64, code UInt64)",
        "CREATE TABLE labels (code UInt64, label String)",
        "INSERT INTO facts VALUES (1, 10, 2), (2, 20, 3)",
        "INSERT INTO accounts VALUES (10, 'Ada'), (20, 'Bob')",
        "INSERT INTO regions VALUES (2, 7)",
        "INSERT INTO labels VALUES (7, 'East')",
    ] {
        let result = sql(statement, &db, &[]);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let joined = sql(
        "SELECT f.id, l.label FROM facts f JOIN accounts a ON f.account = a.id JOIN regions r ON f.region = r.id JOIN labels l ON r.code = l.code ORDER BY f.id",
        &db,
        &[],
    );
    assert!(joined.status.success(), "{joined:?}");
    assert_eq!(joined.stdout, b"f.id\tl.label\n1\tEast\n");
    fs::remove_dir_all(db).unwrap();
}

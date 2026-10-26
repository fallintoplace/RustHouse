use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-multi-join-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(statement: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn composite_keys_preserve_tenant_isolation_and_null_semantics() {
    let db = database();
    for statement in [
        "CREATE TABLE orders (id UInt64, tenant UInt64, customer Nullable(UInt64))",
        "CREATE TABLE customers (tenant UInt64, customer Nullable(UInt64), name String)",
        "INSERT INTO orders VALUES (1, 1, 7), (2, 2, 7), (3, 1, 8), (4, 1, NULL)",
        "INSERT INTO customers VALUES (1, 7, 'Ada'), (2, 7, 'Bob'), (1, 7, 'Amy'), (1, NULL, 'Null')",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let inner = sql(
        "SELECT o.id, c.name FROM orders o INNER JOIN customers c ON o.tenant = c.tenant AND c.customer = o.customer ORDER BY o.id LIMIT 10",
        &db,
    );
    assert!(inner.status.success(), "{inner:?}");
    assert_eq!(inner.stdout, b"o.id\tc.name\n1\tAda\n1\tAmy\n2\tBob\n");
    let left = sql(
        "SELECT o.id, c.name FROM orders o LEFT OUTER JOIN customers c ON o.tenant = c.tenant AND o.customer = c.customer ORDER BY o.id LIMIT 10",
        &db,
    );
    assert!(left.status.success(), "{left:?}");
    assert_eq!(
        left.stdout,
        b"o.id\tc.name\n1\tAda\n1\tAmy\n2\tBob\n3\t\\N\n4\t\\N\n"
    );
    fs::remove_dir_all(db).unwrap();
}

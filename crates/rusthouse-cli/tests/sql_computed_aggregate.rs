use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn computes_single_aggregate_with_arithmetic_argument() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-tpch-single-aggregate-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&database).unwrap();
    for statement in [
        "CREATE TABLE lineitem (l_extendedprice UInt64, l_discount UInt64)",
        "INSERT INTO lineitem VALUES (100, 5), (200, 7), (300, 9)",
    ] {
        let output = sql(statement, &database);
        assert!(output.status.success(), "{statement}: {output:?}");
    }

    let sum = sql(
        "SELECT SUM(l_extendedprice * l_discount) AS revenue FROM lineitem WHERE l_discount <= 7",
        &database,
    );
    assert!(sum.status.success(), "{sum:?}");
    assert_eq!(sum.stdout, b"revenue\n1900\n");

    let count = sql(
        "SELECT COUNT(l_extendedprice * l_discount) AS n FROM lineitem",
        &database,
    );
    assert!(count.status.success(), "{count:?}");
    assert_eq!(count.stdout, b"n\n3\n");

    fs::remove_dir_all(database).unwrap();
}

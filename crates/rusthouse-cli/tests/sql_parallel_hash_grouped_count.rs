use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, db: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap();
    assert!(output.status.success(), "{statement}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn parallel_hash_grouped_counts_preserve_exact_keys_and_checked_totals() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-hash-group-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE facts (account UInt64, signed_key Int32)", &db);
    for index in 0..40 {
        let account = 4_294_967_300u64 + (index % 4) as u64;
        let signed_key = (index % 4) - 2;
        sql(&format!(
            "INSERT INTO facts VALUES ({account}, {signed_key}), ({account}, {signed_key})"
        ), &db);
    }
    assert_eq!(
        sql("SELECT account, count(*) FROM facts GROUP BY account ORDER BY account", &db),
        "account\tcount\n4294967300\t20\n4294967301\t20\n4294967302\t20\n4294967303\t20\n"
    );
    assert_eq!(
        sql("SELECT signed_key, count(*) FROM facts GROUP BY signed_key ORDER BY signed_key", &db),
        "signed_key\tcount\n-2\t20\n-1\t20\n0\t20\n1\t20\n"
    );
    fs::remove_dir_all(db).unwrap();
}

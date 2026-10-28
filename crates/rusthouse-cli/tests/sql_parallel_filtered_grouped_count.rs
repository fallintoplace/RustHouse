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
fn parallel_grouped_counts_filter_before_aggregating_across_all_parts() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-filtered-parallel-group-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE facts (category UInt16, account UInt64, flag UInt8)", &db);
    for index in 0..40u64 {
        let category = index % 4;
        let account = 4_294_967_300 + category;
        sql(&format!(
            "INSERT INTO facts VALUES ({category}, {account}, 1), ({category}, {account}, 0)"
        ), &db);
    }
    for query in [
        "SELECT category, count(*) FROM facts WHERE flag = 1 GROUP BY category ORDER BY category",
        "SELECT category, count(*) FROM facts WHERE flag IN (1) GROUP BY category ORDER BY category",
    ] {
        assert_eq!(sql(query, &db), "category\tcount\n0\t10\n1\t10\n2\t10\n3\t10\n");
    }
    assert_eq!(
        sql("SELECT account, count(*) FROM facts WHERE flag = 1 GROUP BY account ORDER BY account", &db),
        "account\tcount\n4294967300\t10\n4294967301\t10\n4294967302\t10\n4294967303\t10\n"
    );
    assert_eq!(
        sql("SELECT category, count(*) FROM facts WHERE flag = 3 GROUP BY category ORDER BY category", &db),
        "category\tcount\n"
    );
    fs::remove_dir_all(db).unwrap();
}

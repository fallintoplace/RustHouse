use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, db: &Path) -> String {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"]).arg(db).output().unwrap();
    assert!(result.status.success(), "{statement}: {result:?}");
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn parallel_multidimensional_grouping_reduces_intersecting_part_keys() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-parallel-multidim-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    sql("CREATE TABLE events (region UInt8, category String, active UInt8)", &db);
    for index in 0..40 {
        let region = index % 4;
        let category = if index % 2 == 0 { "alpha" } else { "beta" };
        sql(&format!(
            "INSERT INTO events VALUES ({region}, '{category}', 1), ({region}, '{category}', 0)"
        ), &db);
    }
    assert_eq!(
        sql("SELECT region, category, count(*) FROM events GROUP BY region, category ORDER BY region, category", &db),
        "region\tcategory\tcount\n0\talpha\t20\n1\tbeta\t20\n2\talpha\t20\n3\tbeta\t20\n"
    );
    assert_eq!(
        sql("SELECT region, category, count(*) FROM events WHERE active = 1 GROUP BY region, category ORDER BY region, category", &db),
        "region\tcategory\tcount\n0\talpha\t10\n1\tbeta\t10\n2\talpha\t10\n3\tbeta\t10\n"
    );
    fs::remove_dir_all(db).unwrap();
}

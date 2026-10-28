use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(query: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn multidimensional_distinct_byte_bitmap_is_exact_across_parts() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-multi-byte-distinct-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for query in [
        "CREATE TABLE events (region UInt8, segment UInt8, value Nullable(UInt8), signed Nullable(Int8))",
        "INSERT INTO events VALUES (1, 2, 0, -128), (1, 2, 255, 127), (1, 3, 99, NULL), (2, 2, NULL, -1)",
        "INSERT INTO events VALUES (1, 2, 255, -128), (1, 2, NULL, NULL), (1, 3, 99, 0), (2, 2, NULL, -1)",
    ] {
        let result = sql(query, &database);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT region, segment, count(DISTINCT value) FROM events GROUP BY region, segment ORDER BY region, segment",
            "region\tsegment\tcountDistinct(value)\n1\t2\t2\n1\t3\t1\n2\t2\t0\n",
        ),
        (
            "SELECT region, segment, count(DISTINCT signed) FROM events GROUP BY region, segment ORDER BY region, segment",
            "region\tsegment\tcountDistinct(signed)\n1\t2\t2\n1\t3\t1\n2\t2\t1\n",
        ),
    ] {
        let result = sql(query, &database);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{query}");
    }
    fs::remove_dir_all(database).unwrap();
}

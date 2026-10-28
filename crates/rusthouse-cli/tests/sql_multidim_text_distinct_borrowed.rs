use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn exact_distinct_composite_strings_deduplicate_repeated_values_across_parts() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-multidim-text-distinct-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for query in [
        "CREATE TABLE hits (region Nullable(String), segment String, visitor Nullable(String))",
        "INSERT INTO hits VALUES ('a', 'bc', 'x'), ('ab', 'c', 'x'), (NULL, 'bc', NULL), ('a', 'bc', 'y')",
        "INSERT INTO hits VALUES ('a', 'bc', 'x'), ('ab', 'c', 'z'), (NULL, 'bc', 'v'), ('a', 'bc', NULL)",
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    let result = sql(
        "SELECT region, segment, count(DISTINCT visitor) FROM hits GROUP BY region, segment"
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"region\tsegment\tcountDistinct(visitor)\na\tbc\t2\nab\tc\t2\nNULL\tbc\t1\n"
    );
    fs::remove_dir_all(db).unwrap();
}

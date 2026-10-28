use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(query: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn null_safe_comparisons_handle_text_numeric_and_null_values() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-distinct-from-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for query in [
        "CREATE TABLE records (id UInt64, tag Nullable(String), amount Nullable(Int64))",
        "INSERT INTO records VALUES (1, NULL, NULL), (2, 'a', 10), (3, 'b', 10), (4, 'a', 20)",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT id FROM records WHERE tag IS DISTINCT FROM 'a' ORDER BY id",
            "id\n1\n3\n",
        ),
        (
            "SELECT id FROM records WHERE tag IS NOT DISTINCT FROM 'a' ORDER BY id",
            "id\n2\n4\n",
        ),
        (
            "SELECT id FROM records WHERE tag IS DISTINCT FROM NULL ORDER BY id",
            "id\n2\n3\n4\n",
        ),
        (
            "SELECT id FROM records WHERE tag IS NOT DISTINCT FROM NULL",
            "id\n1\n",
        ),
        (
            "SELECT id FROM records WHERE amount IS DISTINCT FROM 10 ORDER BY id",
            "id\n1\n4\n",
        ),
        (
            "SELECT id FROM records WHERE amount IS NOT DISTINCT FROM 10 ORDER BY id",
            "id\n2\n3\n",
        ),
        (
            "SELECT id FROM records WHERE NOT amount IS DISTINCT FROM 10 ORDER BY id",
            "id\n2\n3\n",
        ),
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            expected,
            "{query}"
        );
    }
    for (query, expected) in [
        (
            "SELECT number FROM numbers(5) WHERE number IS DISTINCT FROM 3",
            "number\n0\n1\n2\n4\n",
        ),
        (
            "SELECT number FROM numbers(5) WHERE number IS NOT DISTINCT FROM 3",
            "number\n3\n",
        ),
        (
            "SELECT number FROM numbers(5) WHERE number IS DISTINCT FROM NULL",
            "number\n0\n1\n2\n3\n4\n",
        ),
        (
            "SELECT number FROM numbers(5) WHERE number IS NOT DISTINCT FROM NULL",
            "number\n",
        ),
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            expected,
            "{query}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn rejects_incomplete_distinct_from_syntax() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-distinct-invalid-{}", std::process::id()));
    for query in [
        "SELECT number FROM numbers(3) WHERE number IS DISTINCT 2",
        "SELECT number FROM numbers(3) WHERE number IS NOT DISTINCT",
        "SELECT number FROM numbers(3) WHERE number IS DISTINCT FROM",
        "SELECT number FROM numbers(3) WHERE number NOT IS DISTINCT FROM 2",
    ] {
        let result = sql(query, &db);
        assert!(!result.status.success(), "{query}: {result:?}");
    }
}

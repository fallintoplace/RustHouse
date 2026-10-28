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
fn aggregates_by_two_dimensions_with_nullable_metrics() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-multi-aggregate-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for query in [
        "CREATE TABLE sales (region String, category String, amount Nullable(UInt64))",
        "INSERT INTO sales VALUES ('EU', 'books', 10), ('EU', 'books', 20), ('EU', 'games', 7), ('US', 'books', 5), ('US', 'games', NULL), ('US', 'games', 3)",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
    }

    let result = sql(
        "SELECT region, category, sum(amount), min(amount), max(amount) FROM sales GROUP BY region, category ORDER BY region, category",
        &db,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"region\tcategory\tsum(amount)\tmin(amount)\tmax(amount)\nEU\tbooks\t30\t10\t20\nEU\tgames\t7\t7\t7\nUS\tbooks\t5\t5\t5\nUS\tgames\t3\t3\t3\n"
    );

    let filtered = sql(
        "SELECT region, category, sum(amount) FROM sales WHERE amount IS NOT NULL GROUP BY region, category ORDER BY region DESC, category LIMIT 2",
        &db,
    );
    assert!(filtered.status.success(), "{filtered:?}");
    assert_eq!(
        filtered.stdout,
        b"region\tcategory\tsum(amount)\nUS\tbooks\t5\nUS\tgames\t3\n"
    );
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn rejects_mismatched_grouping_keys() {
    let db = std::env::temp_dir().join(format!("rusthouse-multikey-invalid-{}", std::process::id()));
    let bad = sql(
        "SELECT region, category, sum(amount) FROM sales GROUP BY region",
        &db,
    );
    assert!(!bad.status.success(), "{bad:?}");
}

#[test]
fn repeated_composite_keys_across_parts_keep_distinct_groups() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-multikey-repeated-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for query in [
        "CREATE TABLE sales (region String, category String, amount Nullable(UInt64))",
        "INSERT INTO sales VALUES ('EU', 'books', 10), ('EU', 'books', NULL), ('US', 'games', 3)",
        "INSERT INTO sales VALUES ('EU', 'books', 5), ('US', 'games', 7), ('EU', 'games', 2)",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    let result = sql(
        "SELECT region, category, sum(amount), min(amount), max(amount) FROM sales GROUP BY region, category ORDER BY region, category",
        &db,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"region\tcategory\tsum(amount)\tmin(amount)\tmax(amount)\nEU\tbooks\t15\t5\t10\nEU\tgames\t2\t2\t2\nUS\tgames\t10\t3\t7\n"
    );
    fs::remove_dir_all(db).unwrap();
}

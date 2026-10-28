use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(query: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn filters_grouped_numeric_results_after_aggregation() {
    let db = std::env::temp_dir().join(format!("rusthouse-numeric-having-{}", std::process::id()));
    for query in [
        "CREATE TABLE sales (category String, amount Nullable(UInt64))",
        "INSERT INTO sales VALUES ('EU', 10), ('EU', 20), ('EU', NULL), ('US', 3), ('US', 7), ('APAC', NULL)",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT category, sum(amount) FROM sales GROUP BY category HAVING sum(amount) > 15 ORDER BY category",
            "category\tsum(amount)\nEU\t30\n",
        ),
        (
            "SELECT category, avg(amount) FROM sales GROUP BY category HAVING avg(amount) >= 5 ORDER BY category",
            "category\tavg(amount)\nEU\t15\nUS\t5\n",
        ),
        (
            "SELECT category, min(amount) FROM sales GROUP BY category HAVING min(amount) >= 5 ORDER BY category",
            "category\tmin(amount)\nEU\t10\n",
        ),
        (
            "SELECT category, max(amount) FROM sales GROUP BY category HAVING max(amount) >= 7 ORDER BY category",
            "category\tmax(amount)\nEU\t20\nUS\t7\n",
        ),
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{query}");
    }

    let generated = sql(
        "SELECT number, sum(number) FROM numbers(5) GROUP BY number HAVING sum(number) >= 3 ORDER BY number",
        &db,
    );
    assert!(generated.status.success(), "{generated:?}");
    assert_eq!(
        generated.stdout,
        b"number\tsum(number)\n3\t3\n4\t4\n"
    );
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn rejects_having_an_unselected_aggregate() {
    let db = std::env::temp_dir().join(format!("rusthouse-having-invalid-{}", std::process::id()));
    for query in [
        "SELECT number, sum(number) FROM numbers(5) GROUP BY number HAVING max(number) > 1",
        "SELECT number, sum(number) FROM numbers(5) GROUP BY number HAVING sum(other) > 1",
        "SELECT number, median(number) FROM numbers(5) GROUP BY number HAVING median(number) > 1",
    ] {
        let result = sql(query, &db);
        assert!(!result.status.success(), "{query}: {result:?}");
    }
}

#[test]
fn groups_nullable_strings_with_repeated_metrics() {
    let db = std::env::temp_dir().join(format!("rusthouse-nullable-group-multi-{}", std::process::id()));
    for query in [
        "CREATE TABLE sales (category Nullable(String), amount UInt64)",
        "INSERT INTO sales VALUES ('books', 4), ('books', 6), ('games', 5), (NULL, 1)",
        "INSERT INTO sales VALUES ('books', 3), (NULL, 2)",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    let result = sql(
        "SELECT category, sum(amount), min(amount) FROM sales GROUP BY category ORDER BY category NULLS FIRST",
        &db,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"category\tsum(amount)\tmin(amount)\n\\N\t3\t1\nbooks\t13\t3\ngames\t5\t5\n"
    );
    fs::remove_dir_all(db).unwrap();
}

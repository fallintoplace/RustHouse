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
fn orders_grouped_sum_and_avg_by_result_value() {
    let db = std::env::temp_dir().join(format!("rusthouse-order-metrics-{}", std::process::id()));
    for query in [
        "CREATE TABLE sales (category String, amount Nullable(UInt64))",
        "INSERT INTO sales VALUES ('EU', 10), ('EU', 20), ('US', 5), ('US', 7), ('AU', 12), ('APAC', NULL)",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
    }

    let top = sql(
        "SELECT category, sum(amount) FROM sales GROUP BY category ORDER BY 2 DESC, category ASC LIMIT 3",
        &db,
    );
    assert!(top.status.success(), "{top:?}");
    assert_eq!(
        top.stdout,
        b"category\tsum(amount)\nEU\t30\nAU\t12\nUS\t12\n"
    );

    let nulls = sql(
        "SELECT category, sum(amount) FROM sales GROUP BY category ORDER BY 2 ASC NULLS FIRST",
        &db,
    );
    assert!(nulls.status.success(), "{nulls:?}");
    assert_eq!(
        nulls.stdout,
        b"category\tsum(amount)\nAPAC\tNULL\nAU\t12\nUS\t12\nEU\t30\n"
    );

    let averages = sql(
        "SELECT category, avg(amount) FROM sales GROUP BY category ORDER BY 2 DESC LIMIT 2",
        &db,
    );
    assert!(averages.status.success(), "{averages:?}");
    assert_eq!(
        averages.stdout,
        b"category\tavg(amount)\nEU\t15\nAU\t12\n"
    );
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn orders_generated_grouped_values_by_sum() {
    let db = std::env::temp_dir().join(format!("rusthouse-order-metrics-generated-{}", std::process::id()));
    let result = sql(
        "SELECT number, sum(number) FROM numbers(5) GROUP BY number ORDER BY 2 DESC LIMIT 2",
        &db,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"number\tsum(number)\n4\t4\n3\t3\n");
}

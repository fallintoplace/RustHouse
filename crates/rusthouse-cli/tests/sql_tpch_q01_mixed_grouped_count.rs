use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run(db: &Path, query: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn q1_mixed_grouped_aggregates_count_all_rows_including_nulls() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-tpch-q1-mixed-count-{}", std::process::id()
    ));
    fs::create_dir_all(&db).unwrap();
    for query in [
        "CREATE TABLE facts (region String, category String, amount Nullable(UInt64))",
        "INSERT INTO facts VALUES ('US','A',10), ('US','A',NULL), ('EU','B',5)",
    ] {
        let output = run(&db, query);
        assert!(output.status.success(), "{query}: {output:?}");
    }
    let output = run(
        &db,
        "SELECT region, category, sum(amount) AS total, count(*) AS rows, count(amount) AS known          FROM facts GROUP BY region, category ORDER BY region, category",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        b"region	category	total	rows	known
EU	B	5	1	1
US	A	10	2	1
"
    );
    let output = run(
        &db,
        "SELECT region, sum(amount) AS total, count(*) AS rows          FROM facts GROUP BY region ORDER BY region",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"region	total	rows
EU	5	1
US	10	2
");
    fs::remove_dir_all(&db).unwrap();
}

use std::fs;
use std::process::Command;

#[test]
fn mixed_grouped_distinct_counts_skip_nulls_and_duplicates() {
    let db = std::env::temp_dir().join(format!("rusthouse-mixed-distinct-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE hits (region UInt64, device UInt64, value UInt64, user Nullable(UInt64))",
        "INSERT INTO hits VALUES (1,1,10,3), (1,1,20,3), (1,2,5,4), (2,1,8,NULL), (2,1,2,NULL)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT region, SUM(value) AS total, COUNT(*) AS n, COUNT(DISTINCT user) AS users FROM hits GROUP BY region ORDER BY region",
            "region\ttotal\tn\tusers\n1\t35\t3\t2\n2\t10\t2\t0\n",
        ),
        (
            "SELECT region, device, SUM(value) AS total, COUNT(DISTINCT user) AS users FROM hits GROUP BY region, device ORDER BY region, device",
            "region\tdevice\ttotal\tusers\n1\t1\t30\t1\n1\t2\t5\t1\n2\t1\t10\t0\n",
        ),
        (
            "SELECT region, SUM(value) AS total, COUNT(DISTINCT user) AS users, COUNT(DISTINCT device) AS devices FROM hits GROUP BY region ORDER BY region",
            "region\ttotal\tusers\tdevices\n1\t35\t2\t2\n2\t10\t0\t1\n",
        ),
    ] {
        let output = sql(query);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
    let first = sql(
        "SELECT region, COUNT(DISTINCT user) AS users, SUM(value) AS total FROM hits GROUP BY region ORDER BY region",
    );
    assert!(first.status.success(), "{first:?}");
    assert_eq!(first.stdout, b"region\tusers\ttotal\n1\t2\t35\n2\t0\t10\n");
    let first_without_alias = sql(
        "SELECT region, COUNT(DISTINCT user), SUM(value) AS total FROM hits GROUP BY region ORDER BY region",
    );
    assert!(
        first_without_alias.status.success(),
        "{first_without_alias:?}"
    );
    assert_eq!(
        first_without_alias.stdout,
        b"region\tcountDistinct(user)\ttotal\n1\t2\t35\n2\t0\t10\n"
    );
    let empty = sql(
        "SELECT region, SUM(value) AS total, COUNT(DISTINCT user) AS users FROM hits WHERE value > 100 GROUP BY region ORDER BY region",
    );
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"region\ttotal\tusers\n");
    let sorted = sql(
        "SELECT region, SUM(value) AS total, COUNT(DISTINCT user) AS users FROM hits GROUP BY region ORDER BY users DESC LIMIT 1",
    );
    assert!(sorted.status.success(), "{sorted:?}");
    assert_eq!(sorted.stdout, b"region\ttotal\tusers\n1\t35\t2\n");
    for statement in [
        "CREATE TABLE words (category UInt64, value UInt64, word Nullable(String))",
        "INSERT INTO words VALUES (1,2,'a'), (1,3,'a'), (1,5,'b'), (2,7,NULL)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let output = sql(
        "SELECT category, SUM(value) AS total, COUNT(DISTINCT word) AS words FROM words GROUP BY category ORDER BY category",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        b"category\ttotal\twords\n1\t10\t2\n2\t7\t0\n"
    );
    let generated =
        sql("SELECT number, SUM(number), COUNT(DISTINCT number) FROM numbers(3) GROUP BY number");
    assert!(!generated.status.success(), "{generated:?}");
    assert!(String::from_utf8_lossy(&generated.stderr).contains("requires a stored table"));
    fs::remove_dir_all(db).unwrap();
}

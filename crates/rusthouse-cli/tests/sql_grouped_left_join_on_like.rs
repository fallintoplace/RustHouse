use std::fs;
use std::process::Command;

#[test]
fn grouped_left_join_applies_like_before_null_extension() {
    let db = std::env::temp_dir().join(format!("rusthouse-left-on-like-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE a (id UInt64)",
        "CREATE TABLE b (id UInt64, value Nullable(UInt64), comment Nullable(String))",
        "INSERT INTO a VALUES (1), (2), (3), (4)",
        "INSERT INTO b VALUES (1, 7, 'special urgent requests'), (1, 8, 'good'), (2, 9, NULL), (4, 10, 'special requests')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT a.id, COUNT(b.value) AS n FROM a LEFT JOIN b ON a.id = b.id AND b.comment NOT LIKE '%special%requests%' GROUP BY a.id ORDER BY a.id";
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"a.id\tn\n1\t1\n2\t0\n3\t0\n4\t0\n");
    let positive = query.replace("NOT LIKE", "LIKE");
    let result = sql(&positive);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"a.id\tn\n1\t1\n2\t0\n3\t0\n4\t1\n");
    let any = query.replace("LEFT JOIN", "ANY LEFT JOIN");
    let result = sql(&any);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"a.id\tn\n1\t1\n2\t0\n3\t0\n4\t0\n");
    let left_count = query.replace("COUNT(b.value)", "COUNT(a.id)");
    let result = sql(&left_count);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"a.id\tn\n1\t1\n2\t1\n3\t1\n4\t1\n");
    let right_group = query
        .replace(
            "SELECT a.id, COUNT(b.value)",
            "SELECT b.value, COUNT(b.value)",
        )
        .replace(
            "GROUP BY a.id ORDER BY a.id",
            "GROUP BY b.value ORDER BY b.value NULLS LAST",
        );
    let result = sql(&right_group);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"b.value\tn\n8\t1\n\\N\t0\n");
    assert!(
        sql("INSERT INTO b VALUES (1, 11, '100%_ok'), (1, 12, 'good')")
            .status
            .success()
    );
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"a.id\tn\n1\t3\n2\t0\n3\t0\n4\t0\n");
    let escaped = query.replace("NOT LIKE '%special%requests%'", "LIKE '100\\\\%\\\\_ok'");
    let result = sql(&escaped);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"a.id\tn\n1\t1\n2\t0\n3\t0\n4\t0\n");
    for invalid in [
        query.replace("b.comment NOT LIKE", "b.value NOT LIKE"),
        query.replace("b.comment NOT LIKE", "a.id NOT LIKE"),
        query.replace("b.comment NOT LIKE", "b.missing NOT LIKE"),
        query.replace("COUNT(b.value)", "COUNT(*)"),
        query.replace("LEFT JOIN", "RIGHT JOIN"),
        query.replace("GROUP BY", "AND b.id = a.id GROUP BY"),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    let limit_zero = query
        .replace("b.comment NOT LIKE", "b.value NOT LIKE")
        .replace("ORDER BY a.id", "LIMIT 0");
    assert!(!sql(&limit_zero).status.success());
    fs::remove_dir_all(db).unwrap();
}

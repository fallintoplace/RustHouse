use std::fs;
use std::process::Command;

#[test]
fn q17_filters_by_per_part_average() {
    let db = std::env::temp_dir().join(format!("rusthouse-q17-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE part (p_partkey UInt64, p_brand String, p_container String)",
        "CREATE TABLE lineitem (l_partkey Nullable(UInt64), l_quantity Nullable(Float64), l_extendedprice Float64)",
        "CREATE TABLE bad_lineitem (l_partkey UInt32, l_quantity Float64)",
        "INSERT INTO part VALUES (1, 'Brand#23', 'MED BOX'), (2, 'Brand#23', 'MED BOX'), (3, 'Other', 'MED BOX')",
        "INSERT INTO lineitem VALUES (1, 1, 70), (1, 20, 14), (1, 30, 21), (2, NULL, 1000), (2, 10, 7), (2, 10, 14), (3, 1, 700), (3, 30, 7), (NULL, 1, 7000)",
        "INSERT INTO bad_lineitem VALUES (1, 2)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let query = include_str!("../../../bench/tpch/queries/q17.sql");
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"avg_yearly\n10\n");
    let explicit = query.replace(
        "lineitem,\n    part",
        "lineitem JOIN part ON lineitem.l_partkey = part.p_partkey",
    );
    let result = sql(&explicit);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"avg_yearly\n10\n");
    let more = query.replace("0.2 * avg", "2 * avg");
    let result = sql(&more);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"avg_yearly\n18\n");
    let none = query.replace("'Brand#23'", "'Missing'");
    let result = sql(&none);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"avg_yearly\n\\N\n");
    let limited = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 0";
    let result = sql(&limited);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"avg_yearly\n");
    let offset = query.trim_end().trim_end_matches(';').to_owned() + " OFFSET 1";
    let result = sql(&offset);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"avg_yearly\n");
    let incompatible = query.replace(
        "FROM\n            lineitem",
        "FROM\n            bad_lineitem",
    );
    assert!(!sql(&incompatible).status.success());
    assert!(
        !sql(&query.replace("avg(l_quantity)", "avg(missing)"))
            .status
            .success()
    );
    assert!(
        !sql(&query.replace("l_partkey = p_partkey)", "l_partkey > p_partkey)"))
            .status
            .success()
    );
    fs::remove_dir_all(db).unwrap();
}

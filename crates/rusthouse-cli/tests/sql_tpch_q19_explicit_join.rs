use std::fs;
use std::process::Command;

#[test]
fn q19_boolean_filters_and_comma_join_match_each_row_once() {
    let db = std::env::temp_dir().join(format!("rusthouse-tpch-q19-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE part (p_partkey UInt64, p_brand String, p_container String, p_size Int64)",
        "CREATE TABLE lineitem (l_partkey UInt64, l_extendedprice Float64, l_discount Float64, l_quantity Float64, l_shipmode String, l_shipinstruct String)",
        "INSERT INTO part VALUES (1, 'Brand#12', 'SM BOX', 5), (2, 'Brand#23', 'MED BAG', 10), (3, 'Brand#34', 'LG PACK', 15)",
        "INSERT INTO lineitem VALUES (1, 100, 0.1, 11, 'AIR', 'DELIVER IN PERSON'), (2, 200, 0.25, 20, 'AIR REG', 'DELIVER IN PERSON'), (3, 50, 0, 30, 'AIR', 'DELIVER IN PERSON'), (1, 999, 0, 12, 'AIR', 'DELIVER IN PERSON'), (3, 999, 0, 30, 'SHIP', 'DELIVER IN PERSON')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = include_str!("../../../bench/tpch/queries/q19.sql");
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"revenue\n290\n");
    for statement in [
        "CREATE TABLE left_rows (a_id Nullable(UInt64), amount UInt64)",
        "CREATE TABLE right_rows (b_id Nullable(UInt64), tag String)",
        "INSERT INTO left_rows VALUES (1, 10), (NULL, 1000)",
        "INSERT INTO right_rows VALUES (1, 'a'), (1, 'b'), (NULL, 'a')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let overlap = "SELECT SUM(amount * 1) AS total FROM left_rows, right_rows WHERE (a_id = b_id AND tag = 'a') OR (b_id = a_id AND amount >= 0)";
    let result = sql(overlap);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"total\n20\n");
    let no_common_key = "SELECT SUM(amount * 1) AS total FROM left_rows, right_rows WHERE (a_id = b_id AND tag = 'a') OR tag = 'b'";
    assert!(!sql(no_common_key).status.success());
    fs::remove_dir_all(db).unwrap();
}

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
fn tpch_q16_grouped_distinct_metric_alias() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-tpch-q16-distinct-alias-{}", std::process::id()
    ));
    fs::create_dir_all(&db).unwrap();
    for query in [
        "CREATE TABLE parts (p_brand String, p_type String, p_size Int64, ps_suppkey UInt64)",
        "INSERT INTO parts VALUES ('A','X',1,1), ('A','X',1,2), ('A','X',1,1), ('B','Y',2,3)",
    ] {
        let output = run(&db, query);
        assert!(output.status.success(), "{query}: {output:?}");
    }
    let output = run(
        &db,
        "SELECT p_brand, p_type, p_size, count(DISTINCT ps_suppkey) AS supplier_cnt          FROM parts GROUP BY p_brand, p_type, p_size          ORDER BY supplier_cnt DESC, p_brand, p_type, p_size",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        b"p_brand\tp_type\tp_size\tsupplier_cnt\nA\tX\t1\t2\nB\tY\t2\t1\n"
    );
    fs::remove_dir_all(&db).unwrap();
}

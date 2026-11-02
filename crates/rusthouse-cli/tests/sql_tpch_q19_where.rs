use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run(database: &Path, sql: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", sql, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn canonical_q19_disjunctive_filter_matches_on_joined_row_shape() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-tpch-q19-filters-{}", std::process::id()
    ));
    fs::create_dir_all(&database).unwrap();
    for sql in [
        "CREATE TABLE fused (p_partkey UInt64, l_partkey UInt64, p_brand String, p_container String, l_quantity UInt64, p_size UInt64, l_shipmode String, l_shipinstruct String)",
        "INSERT INTO fused VALUES          (1,1,'Brand#12','SM BOX',4,3,'AIR','DELIVER IN PERSON'),          (2,2,'Brand#23','MED BAG',15,7,'AIR REG','DELIVER IN PERSON'),          (3,3,'Brand#34','LG BOX',25,10,'AIR','DELIVER IN PERSON'),          (4,9,'Brand#34','LG BOX',25,10,'AIR','DELIVER IN PERSON'),          (5,5,'Brand#12','SM BOX',5,2,'TRUCK','DELIVER IN PERSON')",
    ] {
        let output = run(&database, sql);
        assert!(output.status.success(), "{sql}: {output:?}");
    }
    let canonical = include_str!("../../../bench/tpch/queries/q19.sql");
    let predicate = canonical
        .split_once("WHERE")
        .unwrap()
        .1
        .trim()
        .trim_end_matches(';');
    let output = run(
        &database,
        &format!("SELECT count(*) FROM fused WHERE {predicate}"),
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"count\n3\n");
    fs::remove_dir_all(&database).unwrap();
}

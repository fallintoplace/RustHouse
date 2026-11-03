use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::process::Command;

#[test]
fn q20_streams_large_partsupp_and_preserves_correlated_sum() {
    let db = std::env::temp_dir().join(format!("rusthouse-q20-large-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE supplier (s_suppkey UInt64, s_nationkey UInt64, s_name String, s_address String)",
        "CREATE TABLE nation (n_nationkey UInt64, n_name String)",
        "CREATE TABLE part (p_partkey UInt64, p_name String)",
        "CREATE TABLE partsupp (ps_suppkey UInt64, ps_partkey UInt64, ps_availqty Int64)",
        "CREATE TABLE lineitem (l_partkey UInt64, l_suppkey UInt64, l_quantity Nullable(Float64), l_shipdate Date)",
        "INSERT INTO nation VALUES (1, 'CANADA')",
        "INSERT INTO supplier VALUES (1, 1, 'A', 'a'), (2, 1, 'B', 'b')",
        "INSERT INTO part VALUES (1, 'forest green'), (2, 'forest red')",
        "INSERT INTO partsupp VALUES (1, 1, 11), (2, 1, 10), (2, 2, 100)",
        "INSERT INTO lineitem VALUES (1, 1, 20, '1994-01-01'), (1, 2, 20, '1994-01-01'), (2, 2, NULL, '1994-01-01')",
    ] {
        let out = sql(statement);
        assert!(
            out.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let data = db.join("filler.tsv");
    let mut file = BufWriter::new(File::create(&data).unwrap());
    for i in 0..33_000 {
        writeln!(file, "1\t{}\t1", 1000 + i).unwrap();
    }
    drop(file);
    let out = sql(&format!(
        "COPY partsupp FROM '{}' WITH (FORMAT TSV)",
        data.display()
    ));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let query = include_str!("../../../bench/tpch/queries/q20.sql");
    let out = sql(query);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"s_name\ts_address\nA\ta\n");
    let zero = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 0";
    assert_eq!(sql(&zero).stdout, b"s_name\ts_address\n");
    fs::remove_dir_all(db).unwrap();
}

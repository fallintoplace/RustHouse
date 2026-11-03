use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::process::Command;

#[test]
fn q2_handles_projected_partsupp_above_materialized_row_limit() {
    let db = std::env::temp_dir().join(format!("rusthouse-q2-large-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE part (p_partkey UInt64, p_size UInt64, p_type String, p_mfgr String)",
        "CREATE TABLE supplier (s_suppkey UInt64, s_nationkey UInt64, s_acctbal Float64, s_name String, s_address String, s_phone String, s_comment String)",
        "CREATE TABLE partsupp (ps_partkey UInt64, ps_suppkey UInt64, ps_supplycost Nullable(Float64))",
        "CREATE TABLE nation (n_nationkey UInt64, n_regionkey UInt64, n_name String)",
        "CREATE TABLE region (r_regionkey UInt64, r_name String)",
        "INSERT INTO part VALUES (1, 15, 'SMALL BRASS', 'M1'), (2, 15, 'BRASS', 'M2')",
        "INSERT INTO region VALUES (1, 'EUROPE')",
        "INSERT INTO nation VALUES (1, 1, 'FRANCE')",
        "INSERT INTO supplier VALUES (11, 1, 50, 'A', 'a', '1', 'aa'), (22, 1, 80, 'B', 'b', '2', 'bb')",
        "INSERT INTO partsupp VALUES (1, 11, 10), (1, 22, 10), (2, 11, NULL)",
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
        writeln!(file, "{}\t11\t20", 1000 + i).unwrap();
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
    let query = include_str!("../../../bench/tpch/queries/q02.sql");
    let out = sql(query);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"s_acctbal\ts_name\tn_name\tp_partkey\tp_mfgr\ts_address\ts_phone\ts_comment\n80\tB\tFRANCE\t1\tM1\tb\t2\tbb\n50\tA\tFRANCE\t1\tM1\ta\t1\taa\n");
    let zero = query.replace("LIMIT 100;", "LIMIT 0;");
    assert_eq!(
        sql(&zero).stdout,
        b"s_acctbal\ts_name\tn_name\tp_partkey\tp_mfgr\ts_address\ts_phone\ts_comment\n"
    );
    assert!(
        !sql(&query.replace("min(ps_supplycost)", "min(missing)"))
            .status
            .success()
    );
    fs::remove_dir_all(db).unwrap();
}

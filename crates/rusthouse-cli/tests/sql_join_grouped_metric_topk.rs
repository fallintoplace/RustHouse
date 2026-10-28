use std::fs;
use std::process::Command;

#[test]
fn joined_grouped_aggregates_rank_final_metric_not_partial_fact_rows() {
    let db = std::env::temp_dir().join(format!("rusthouse-joined-topk-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dims (id UInt64)",
        "INSERT INTO dims SELECT number FROM numbers(8192)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for _ in 0..5 {
        let result = sql("INSERT INTO facts SELECT number FROM numbers(8192)");
        assert!(result.status.success(), "{result:?}");
    }
    for (query, expected) in [
        (
            "SELECT d.id, sum(f.id) FROM facts f JOIN dims d ON f.id = d.id GROUP BY d.id ORDER BY sum(f.id) DESC LIMIT 3",
            b"d.id\tsum(f.id)\n8191\t40955\n8190\t40950\n8189\t40945\n".as_slice(),
        ),
        (
            "SELECT d.id, count(*) FROM facts f JOIN dims d ON f.id = d.id GROUP BY d.id ORDER BY count(*) DESC LIMIT 2 OFFSET 1",
            b"d.id\tcount\n1\t5\n2\t5\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}

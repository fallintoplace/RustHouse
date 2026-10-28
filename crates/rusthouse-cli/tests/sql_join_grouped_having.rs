use std::fs;
use std::process::Command;

#[test]
fn joined_having_filters_complete_counts_and_sums_before_topk() {
    let db = std::env::temp_dir().join(format!("rusthouse-joined-having-{}", std::process::id()));
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
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for _ in 0..5 {
        let output = sql("INSERT INTO facts SELECT number FROM numbers(8192)");
        assert!(output.status.success(), "{output:?}");
    }
    for (query, expected) in [
        (
            "SELECT d.id, sum(f.id) FROM facts f JOIN dims d ON f.id = d.id GROUP BY d.id HAVING sum(f.id) >= 40950 ORDER BY 2 DESC LIMIT 2",
            b"d.id\tsum(f.id)\n8191\t40955\n8190\t40950\n".as_slice(),
        ),
        (
            "SELECT d.id, count(*) FROM facts f JOIN dims d ON f.id = d.id GROUP BY d.id HAVING count(*) > 4 ORDER BY count(*) DESC LIMIT 3",
            b"d.id\tcount\n0\t5\n1\t5\n2\t5\n",
        ),
        (
            "SELECT d.id, count(*) FROM facts f JOIN dims d ON f.id = d.id GROUP BY d.id HAVING count(*) > 5",
            b"d.id\tcount\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}

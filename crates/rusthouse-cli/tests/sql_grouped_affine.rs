use std::fs;
use std::process::Command;

#[test]
fn grouped_arithmetic_uses_the_base_integer_key() {
    let db = std::env::temp_dir().join(format!("rusthouse-affine-group-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for stmt in [
        "CREATE TABLE hits (ClientIP Nullable(Int64))",
        "INSERT INTO hits VALUES (0), (0), (3), (-1), (NULL)",
    ] {
        let output = sql(stmt);
        assert!(output.status.success(), "{stmt}: {output:?}");
    }
    let query = "SELECT ClientIP, ClientIP - 1, ClientIP - 2, ClientIP - 3, COUNT(*) AS c FROM hits GROUP BY ClientIP, ClientIP - 1, ClientIP - 2, ClientIP - 3 ORDER BY c DESC, ClientIP NULLS LAST LIMIT 10";
    let output = sql(query);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"ClientIP\tClientIP - 1\tClientIP - 2\tClientIP - 3\tc\n0\t-1\t-2\t-3\t2\n-1\t-2\t-3\t-4\t1\n3\t2\t1\t0\t1\n\\N\t\\N\t\\N\t\\N\t1\n");
    let zero = query.replace("LIMIT 10", "LIMIT 0");
    assert_eq!(
        sql(&zero).stdout,
        b"ClientIP\tClientIP - 1\tClientIP - 2\tClientIP - 3\tc\n"
    );
    assert!(
        !sql(&query.replace("ClientIP - 3 ORDER", "ClientIP - 4 ORDER"))
            .status
            .success()
    );
    let plus = "SELECT ClientIP, ClientIP + 0, COUNT(*) AS c FROM hits GROUP BY ClientIP, ClientIP + 0 ORDER BY ClientIP NULLS LAST";
    assert_eq!(
        sql(plus).stdout,
        b"ClientIP\tClientIP + 0\tc\n-1\t-1\t1\n0\t0\t2\n3\t3\t1\n\\N\t\\N\t1\n"
    );
    assert!(
        !sql(&query.replace("ORDER BY c DESC, ClientIP NULLS LAST", "ORDER BY 2"))
            .status
            .success()
    );
    assert!(sql("CREATE TABLE labels (name String)").status.success());
    let text =
        "SELECT name, name - 1, COUNT(*) AS c FROM labels GROUP BY name, name - 1 ORDER BY c";
    assert!(!sql(text).status.success());
    let json = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(&db)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(json.status.success(), "{json:?}");
    assert!(String::from_utf8_lossy(&json.stdout).contains("\"ClientIP - 1\":\"-1\""));
    fs::remove_dir_all(db).unwrap();
}

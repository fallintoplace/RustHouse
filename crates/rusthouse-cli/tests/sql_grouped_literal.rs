use std::fs;
use std::process::Command;

#[test]
fn grouped_literal_projects_for_each_group() {
    let db = std::env::temp_dir().join(format!("rusthouse-group-literal-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for stmt in [
        "CREATE TABLE hits (URL String)",
        "INSERT INTO hits VALUES ('one'), ('one'), ('two')",
    ] {
        let output = sql(stmt);
        assert!(output.status.success(), "{stmt}: {output:?}");
    }
    let query = "SELECT 1, URL, COUNT(*) AS c FROM hits GROUP BY 1, URL ORDER BY c DESC, URL";
    let output = sql(query);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"1\tURL\tc\n1\tone\t2\n1\ttwo\t1\n");
    let positions = query.replace("GROUP BY 1, URL", "GROUP BY 1, 2");
    assert_eq!(sql(&positions).stdout, output.stdout);
    let order_position = query.replace("ORDER BY c DESC, URL", "ORDER BY 2 DESC");
    assert_eq!(
        sql(&order_position).stdout,
        b"1\tURL\tc\n1\ttwo\t1\n1\tone\t2\n"
    );
    assert!(
        !sql(&query.replace("ORDER BY c DESC, URL", "ORDER BY 1"))
            .status
            .success()
    );
    let empty = query.replace("FROM hits", "FROM hits WHERE URL = 'missing'");
    assert_eq!(sql(&empty).stdout, b"1\tURL\tc\n");
    assert_eq!(sql(&(query.to_owned() + " LIMIT 0")).stdout, b"1\tURL\tc\n");
    for invalid in [
        query.replace("GROUP BY 1, URL", "GROUP BY 1, 1"),
        query.replace("GROUP BY 1, URL", "GROUP BY 1, 0"),
        query.replace("GROUP BY 1, URL", "GROUP BY 1, 3"),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    let json = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(&db)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(json.status.success(), "{json:?}");
    assert!(String::from_utf8_lossy(&json.stdout).contains("\"1\":\"1\""));
    fs::remove_dir_all(db).unwrap();
}

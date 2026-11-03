use std::fs;
use std::process::Command;

#[test]
fn grouped_date_trunc_combines_timestamps_before_pagination() {
    let db = std::env::temp_dir().join(format!("rusthouse-minute-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for stmt in [
        "CREATE TABLE hits (EventTime Nullable(DateTime), active UInt8)",
        "INSERT INTO hits VALUES ('2020-01-01 12:00:01', 1), ('2020-01-01 12:00:59', 1), ('2020-01-01 12:01:00', 1), ('2020-01-01 12:01:30', 0), (NULL, 1)",
    ] {
        let output = sql(stmt);
        assert!(output.status.success(), "{stmt}: {output:?}");
    }
    let query = "SELECT DATE_TRUNC('minute', EventTime) AS M, COUNT(*) AS c FROM hits WHERE active = 1 GROUP BY DATE_TRUNC('minute', EventTime) ORDER BY DATE_TRUNC('minute', EventTime) ASC NULLS LAST";
    let out = sql(query);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout,
        b"M\tc\n2020-01-01 12:00:00\t2\n2020-01-01 12:01:00\t1\n\\N\t1\n"
    );
    assert_eq!(
        sql(&(query.to_owned() + " LIMIT 1 OFFSET 1")).stdout,
        b"M\tc\n2020-01-01 12:01:00\t1\n"
    );
    assert_eq!(sql(&(query.to_owned() + " LIMIT 0")).stdout, b"M\tc\n");
    let desc = query.replace(" ASC NULLS LAST", " DESC NULLS LAST");
    assert_eq!(
        sql(&desc).stdout,
        b"M\tc\n2020-01-01 12:01:00\t1\n2020-01-01 12:00:00\t2\n\\N\t1\n"
    );
    assert!(
        !sql(&query.replace("GROUP BY DATE_TRUNC('minute'", "GROUP BY DATE_TRUNC('hour'"))
            .status
            .success()
    );
    fs::remove_dir_all(db).unwrap();
}

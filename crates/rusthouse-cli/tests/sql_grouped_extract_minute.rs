use std::fs;
use std::process::Command;

#[test]
fn extracted_minutes_combine_across_hours_and_preserve_other_keys() {
    let db = std::env::temp_dir().join(format!("rusthouse-extract-minute-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for stmt in [
        "CREATE TABLE hits (UserID Int64, EventTime Nullable(DateTime), SearchPhrase Nullable(String))",
        "INSERT INTO hits VALUES (1, '2020-01-01 12:01:03', 'a'), (1, '2020-01-01 13:01:50', 'a'), (1, '2020-01-01 12:01:00', 'b'), (2, '2020-01-01 12:01:00', 'a'), (1, NULL, NULL)",
    ] {
        let output = sql(stmt);
        assert!(output.status.success(), "{stmt}: {output:?}");
    }
    let q = "SELECT UserID, extract(minute FROM EventTime) AS m, SearchPhrase, COUNT(*) FROM hits GROUP BY UserID, m, SearchPhrase ORDER BY COUNT(*) DESC LIMIT 10";
    let out = sql(q);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout,
        b"UserID\tm\tSearchPhrase\tcount\n1\t1\ta\t2\n1\t1\tb\t1\n1\t\\N\t\\N\t1\n2\t1\ta\t1\n"
    );
    assert_eq!(
        sql(&q.replace("LIMIT 10", "LIMIT 0")).stdout,
        b"UserID\tm\tSearchPhrase\tcount\n"
    );
    assert_eq!(
        sql(&q.replace("LIMIT 10", "LIMIT 1 OFFSET 1")).stdout,
        b"UserID\tm\tSearchPhrase\tcount\n1\t1\tb\t1\n"
    );
    assert!(
        !sql(&q.replace(
            "GROUP BY UserID, m, SearchPhrase",
            "GROUP BY UserID, EventTime, SearchPhrase"
        ))
        .status
        .success()
    );
    let shadowed = q
        .replace("AS m", "AS EventTime")
        .replace("UserID, m, SearchPhrase", "UserID, EventTime, SearchPhrase");
    assert!(!sql(&shadowed).status.success());
    assert!(
        sql("CREATE TABLE float_hits (UserID Float64, EventTime DateTime, SearchPhrase String)")
            .status
            .success()
    );
    let float_q = q.replace("FROM hits", "FROM float_hits");
    assert!(!sql(&float_q).status.success());
    let duplicate_alias = q.replace("COUNT(*) FROM", "COUNT(*) AS m FROM");
    assert!(!sql(&duplicate_alias).status.success());
    let escaped = db.join("escaped.tsv");
    fs::write(
        &escaped,
        b"3\t2020-01-01 12:01:01\tA\\tB\n3\t2020-01-01 13:01:50\tA\\tB\n",
    )
    .unwrap();
    assert!(
        sql(&format!(
            "COPY hits FROM '{}' WITH (FORMAT TSV)",
            escaped.display()
        ))
        .status
        .success()
    );
    let escaped_query = q.replace("FROM hits", "FROM hits WHERE UserID = 3");
    assert_eq!(
        sql(&escaped_query).stdout,
        b"UserID\tm\tSearchPhrase\tcount\n3\t1\tA\\tB\t2\n"
    );
    fs::remove_dir_all(db).unwrap();
}

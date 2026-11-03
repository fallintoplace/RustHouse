use std::fs;
use std::process::Command;

#[test]
fn grouped_min_and_max_of_nullable_strings_with_other_metrics() {
    let db = std::env::temp_dir().join(format!("rusthouse-text-extrema-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE hits (phrase String, url Nullable(String), uid UInt64)",
        "INSERT INTO hits VALUES ('a', 'z', 1), ('a', 'é', 1), ('a', 'a', 2), ('a', NULL, 3), ('b', NULL, 4), ('b', NULL, 4), ('c', '', 5)",
    ] {
        let out = sql(statement);
        assert!(
            out.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let query = "SELECT phrase, MIN(url) AS lo, MAX(url) AS hi, COUNT(*) AS n, COUNT(DISTINCT uid) AS u FROM hits GROUP BY phrase ORDER BY phrase";
    let out = sql(query);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "phrase\tlo\thi\tn\tu\na\ta\té\t4\t3\nb\t\\N\t\\N\t2\t1\nc\t\t\t1\t1\n"
    );
    let single =
        "SELECT phrase, MIN(url) FROM hits GROUP BY phrase ORDER BY MIN(url) DESC NULLS LAST";
    let out = sql(single);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"phrase\tmin(url)\na\ta\nc\t\nb\t\\N\n");
    let ordered = "SELECT phrase, MIN(url) AS lo, COUNT(*) AS n FROM hits GROUP BY phrase ORDER BY lo DESC NULLS LAST";
    assert_eq!(
        sql(ordered).stdout,
        b"phrase\tlo\tn\na\ta\t4\nc\t\t1\nb\t\\N\t2\n"
    );
    let filtered = "SELECT phrase, MIN(url) AS lo, COUNT(*) AS n FROM hits WHERE uid > 1 GROUP BY phrase ORDER BY phrase";
    assert_eq!(
        sql(filtered).stdout,
        b"phrase\tlo\tn\na\ta\t2\nb\t\\N\t2\nc\t\t1\n"
    );
    let zero = format!("{query} LIMIT 0");
    assert_eq!(sql(&zero).stdout, b"phrase\tlo\thi\tn\tu\n");
    let invalid = "SELECT phrase, SUM(url) AS total, COUNT(*) AS n FROM hits GROUP BY phrase";
    assert!(!sql(invalid).status.success());
    let escaped = db.join("escaped.tsv");
    fs::write(&escaped, b"d\tA\\tB\t1\nd\tA\\nB\t2\n").unwrap();
    let out = sql(&format!(
        "COPY hits FROM '{}' WITH (FORMAT TSV)",
        escaped.display()
    ));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let q = "SELECT phrase, MIN(url) AS lo, MAX(url) AS hi FROM hits WHERE phrase = 'd' GROUP BY phrase";
    assert_eq!(sql(q).stdout, b"phrase\tlo\thi\nd\tA\\tB\tA\\nB\n");
    fs::remove_dir_all(db).unwrap();
}

use std::fs;
use std::process::Command;

#[test]
fn grouped_regex_combines_source_rows_and_applies_having_after_grouping() {
    let db = std::env::temp_dir().join(format!("rusthouse-regex-group-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for stmt in [
        "CREATE TABLE hits (Referer Nullable(String))",
        "INSERT INTO hits VALUES ('http://www.a.com/a'), ('https://a.com/b'), ('https://b.org/x'), (''), (NULL)",
    ] {
        let output = sql(stmt);
        assert!(output.status.success(), "{stmt}: {output:?}");
    }
    let q = r"SELECT REGEXP_REPLACE(Referer, '^https?://(?:www\.)?([^/]+)/.*$', '\1') AS k, AVG(length(Referer)) AS l, COUNT(*) AS c, MIN(Referer) FROM hits WHERE Referer <> '' GROUP BY k HAVING COUNT(*) > 1 ORDER BY l DESC LIMIT 25";
    let out = sql(q);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout,
        b"k\tl\tc\tmin(Referer)\na.com\t16.5\t2\thttp://www.a.com/a\n"
    );
    assert_eq!(
        sql(&q.replace("LIMIT 25", "LIMIT 0")).stdout,
        b"k\tl\tc\tmin(Referer)\n"
    );
    let bad = q.replace("'^https?://(?:www\\.)?([^/]+)/.*$'", "'['");
    assert!(!sql(&bad).status.success());
    let shadow = q
        .replace("AS k", "AS Referer")
        .replace("GROUP BY k", "GROUP BY Referer");
    assert!(!sql(&shadow).status.success());
    assert!(
        sql("CREATE TABLE unicode_hits (Referer Nullable(String))")
            .status
            .success()
    );
    assert!(
        sql("INSERT INTO unicode_hits VALUES ('éé'), (NULL)")
            .status
            .success()
    );
    let unicode = "SELECT REGEXP_REPLACE(Referer, '.', 'x') AS k, AVG(length(Referer)) AS l, COUNT(*) AS c, MIN(Referer) FROM unicode_hits GROUP BY k HAVING COUNT(*) > 0 ORDER BY l DESC NULLS LAST";
    let out = sql(unicode);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "k\tl\tc\tmin(Referer)\nxé\t4\t1\téé\n\\N\t\\N\t1\t\\N\n"
    );
    fs::remove_dir_all(db).unwrap();
}

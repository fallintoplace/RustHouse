use std::fs;
use std::process::Command;

#[test]
fn grouped_case_collapses_referers_when_flags_are_not_zero() {
    let db = std::env::temp_dir().join(format!("rusthouse-group-case-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for stmt in [
        "CREATE TABLE hits (traffic Int64, search Nullable(Int64), adv Nullable(Int64), referer Nullable(String), url String)",
        "INSERT INTO hits VALUES (1, 0, 0, 'a', 'x'), (1, 0, 0, 'b', 'x'), (1, 1, 0, 'a', 'x'), (1, 1, 0, 'b', 'x'), (1, NULL, 0, 'c', 'x'), (1, 0, 0, NULL, 'x')",
    ] {
        let output = sql(stmt);
        assert!(output.status.success(), "{stmt}: {output:?}");
    }
    let q = "SELECT traffic, search, adv, CASE WHEN (search = 0 AND adv = 0) THEN referer ELSE '' END AS Src, url AS Dst, COUNT(*) AS PageViews FROM hits GROUP BY traffic, search, adv, Src, Dst ORDER BY PageViews DESC LIMIT 10";
    let out = sql(q);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"traffic\tsearch\tadv\tSrc\tDst\tPageViews\n1\t1\t0\t\tx\t2\n1\t0\t0\t\\N\tx\t1\n1\t0\t0\ta\tx\t1\n1\t0\t0\tb\tx\t1\n1\t\\N\t0\t\tx\t1\n");
    assert_eq!(
        sql(&q.replace("LIMIT 10", "LIMIT 0")).stdout,
        b"traffic\tsearch\tadv\tSrc\tDst\tPageViews\n"
    );
    assert!(
        !sql(&q
            .replace("AS Src", "AS referer")
            .replace("adv, Src, Dst", "adv, referer, Dst"))
        .status
        .success()
    );
    fs::remove_dir_all(db).unwrap();
}

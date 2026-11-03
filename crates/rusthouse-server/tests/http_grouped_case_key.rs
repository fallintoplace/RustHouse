use rusthouse_server::{Config, handle_request};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn grouped_case_key_works_over_http() {
    let root = std::env::temp_dir().join(format!("rusthouse-http-case-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let part = root.join("all_0_0_0");
    let columns = [
        ("traffic", "Int64"),
        ("search", "Int64"),
        ("adv", "Int64"),
        ("referer", "String"),
        ("url", "String"),
    ]
    .iter()
    .map(|(name, kind)| Column {
        name: (*name).into(),
        data_type: (*kind).into(),
    })
    .collect::<Vec<_>>();
    let row = |referer: &str| {
        ["1", "1", "0", referer, "x"]
            .iter()
            .map(|v| Some((*v).into()))
            .collect::<Vec<_>>()
    };
    write_wide_part(&part, &columns, &[row("a"), row("b")]).unwrap();
    let config = Config {
        part: Some(part),
        max_rows: 10,
        ..Config::default()
    };
    let sql = b"SELECT traffic, search, adv, CASE WHEN (search = 0 AND adv = 0) THEN referer ELSE '' END AS Src, url AS Dst, COUNT(*) AS c FROM part GROUP BY traffic, search, adv, Src, Dst ORDER BY c DESC";
    let result = handle_request("POST", "/", sql, &config);
    assert_eq!(
        result.status,
        "200 OK",
        "{}",
        String::from_utf8_lossy(&result.body)
    );
    assert_eq!(
        result.body,
        b"traffic\tsearch\tadv\tSrc\tDst\tc\n1\t1\t0\t\tx\t2\n"
    );
    fs::remove_dir_all(root).unwrap();
}

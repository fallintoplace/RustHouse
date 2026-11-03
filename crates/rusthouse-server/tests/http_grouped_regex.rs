use rusthouse_server::{Config, handle_request};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn grouped_regex_works_over_http() {
    let root = std::env::temp_dir().join(format!("rusthouse-http-regex-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let part = root.join("all_0_0_0");
    write_wide_part(
        &part,
        &[Column {
            name: "Referer".into(),
            data_type: "String".into(),
        }],
        &[
            vec![Some("http://a/x".into())],
            vec![Some("http://a/y".into())],
        ],
    )
    .unwrap();
    let config = Config {
        part: Some(part),
        max_rows: 10,
        ..Config::default()
    };
    let sql = br"SELECT REGEXP_REPLACE(Referer, '^http://([^/]+)/.*$', '\1') AS k, AVG(length(Referer)) AS l, COUNT(*) AS c, MIN(Referer) FROM part WHERE Referer <> '' GROUP BY k HAVING COUNT(*) > 1 ORDER BY l DESC LIMIT 25";
    let result = handle_request("POST", "/", sql, &config);
    assert_eq!(
        result.status,
        "200 OK",
        "{}",
        String::from_utf8_lossy(&result.body)
    );
    assert_eq!(
        result.body,
        b"k\tl\tc\tmin(Referer)\na\t10\t2\thttp://a/x\n"
    );
    fs::remove_dir_all(root).unwrap();
}

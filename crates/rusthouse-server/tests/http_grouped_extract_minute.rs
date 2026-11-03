use rusthouse_server::{Config, handle_request};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn extract_minute_groups_over_http() {
    let root = std::env::temp_dir().join(format!("rusthouse-http-extract-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let part = root.join("all_0_0_0");
    let columns = [
        Column {
            name: "id".into(),
            data_type: "Int64".into(),
        },
        Column {
            name: "t".into(),
            data_type: "DateTime".into(),
        },
        Column {
            name: "s".into(),
            data_type: "String".into(),
        },
    ];
    let row = |time: &str| vec![Some("1".into()), Some(time.into()), Some("a".into())];
    write_wide_part(
        &part,
        &columns,
        &[row("2020-01-01 12:01:01"), row("2020-01-01 13:01:59")],
    )
    .unwrap();
    let config = Config {
        part: Some(part),
        max_rows: 10,
        ..Config::default()
    };
    let sql = b"SELECT id, extract(minute FROM t) AS m, s, COUNT(*) FROM part GROUP BY id, m, s ORDER BY COUNT(*) DESC";
    let result = handle_request("POST", "/", sql, &config);
    assert_eq!(
        result.status,
        "200 OK",
        "{}",
        String::from_utf8_lossy(&result.body)
    );
    assert_eq!(result.body, b"id\tm\ts\tcount\n1\t1\ta\t2\n");
    fs::remove_dir_all(root).unwrap();
}

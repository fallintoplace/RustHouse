use rusthouse_server::{Config, handle_request};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn date_trunc_minute_groups_over_http() {
    let root = std::env::temp_dir().join(format!("rusthouse-http-minute-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let part = root.join("all_0_0_0");
    write_wide_part(
        &part,
        &[Column {
            name: "t".into(),
            data_type: "DateTime".into(),
        }],
        &[
            vec![Some("2020-01-01 12:00:01".into())],
            vec![Some("2020-01-01 12:00:59".into())],
            vec![Some("2020-01-01 12:01:00".into())],
        ],
    )
    .unwrap();
    let config = Config {
        part: Some(part),
        max_rows: 1,
        ..Config::default()
    };
    let sql = b"SELECT DATE_TRUNC('minute', t) AS m, COUNT(*) AS c FROM part GROUP BY DATE_TRUNC('minute', t) ORDER BY DATE_TRUNC('minute', t)";
    let result = handle_request("POST", "/", sql, &config);
    assert_eq!(
        result.status,
        "200 OK",
        "{}",
        String::from_utf8_lossy(&result.body)
    );
    assert_eq!(result.body, b"m\tc\n2020-01-01 12:00:00\t2\n");
    fs::remove_dir_all(root).unwrap();
}

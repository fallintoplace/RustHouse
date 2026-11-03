use rusthouse_server::{Config, handle_request};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn grouped_arithmetic_works_over_http() {
    let root = std::env::temp_dir().join(format!("rusthouse-http-affine-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let part = root.join("all_0_0_0");
    write_wide_part(
        &part,
        &[Column {
            name: "id".into(),
            data_type: "Int64".into(),
        }],
        &[
            vec![Some("1".into())],
            vec![Some("2".into())],
            vec![Some("1".into())],
        ],
    )
    .unwrap();
    let config = Config {
        part: Some(part),
        max_rows: 10,
        ..Config::default()
    };
    let sql = b"SELECT id, id - 1, COUNT(*) AS c FROM part GROUP BY id, id - 1 ORDER BY id";
    let result = handle_request("POST", "/", sql, &config);
    assert_eq!(
        result.status,
        "200 OK",
        "{}",
        String::from_utf8_lossy(&result.body)
    );
    assert_eq!(result.body, b"id\tid - 1\tc\n1\t0\t2\n2\t1\t1\n");
    fs::remove_dir_all(root).unwrap();
}

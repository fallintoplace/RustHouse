use rusthouse_server::{Config, handle_request};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn http_describe_reads_validated_persistent_table_schemas() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-http-describe-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(db.join("events.parts")).unwrap();
    fs::write(
        db.join("events.schema"),
        b"rusthouse-schema-v1\nid\tUInt64\nlabel\tString\n",
    ).unwrap();
    let columns = [
        Column { name: "id".into(), data_type: "UInt64".into() },
        Column { name: "label".into(), data_type: "String".into() },
    ];
    write_wide_part(&db.join("events.parts/all_0_0_0"), &columns, &[]).unwrap();
    let config = Config {
        database: Some(db.clone()),
        part: None,
        max_rows: 100,
    };
    let tsv = handle_request("POST", "/", b"DESCRIBE TABLE events", &config);
    assert_eq!(tsv.status, "200 OK");
    assert_eq!(tsv.body, b"name\ttype\nid\tUInt64\nlabel\tString\n");
    let json = handle_request("POST", "/?format=json", b"DESC TABLE events", &config);
    assert_eq!(json.status, "200 OK");
    assert_eq!(
        json.body,
        b"[{\"name\":\"id\",\"type\":\"UInt64\"},{\"name\":\"label\",\"type\":\"String\"}]\n"
    );
    assert_eq!(
        handle_request("POST", "/", b"DESCRIBE TABLE missing", &config).status,
        "400 Bad Request"
    );
    assert_eq!(
        handle_request("POST", "/", b"DESCRIBE TABLE events; DROP TABLE events", &config).status,
        "400 Bad Request"
    );
    assert_eq!(
        handle_request("POST", "/explain", b"DESCRIBE TABLE events", &config).status,
        "400 Bad Request"
    );
    fs::remove_dir_all(db).unwrap();
}

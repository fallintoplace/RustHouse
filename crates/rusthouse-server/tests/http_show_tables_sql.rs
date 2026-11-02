use rusthouse_server::{Config, handle_request};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn sql_show_tables_uses_read_only_schema_discovery() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-http-show-tables-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&db).unwrap();
    for name in ["logs", "events"] {
        fs::write(
            db.join(format!("{name}.schema")),
            b"rusthouse-schema-v1\nid\tUInt64\n",
        ).unwrap();
        fs::create_dir(db.join(format!("{name}.parts"))).unwrap();
    }
    let config = Config {
        database: Some(db.clone()),
        part: None,
        max_rows: 100,
    };
    let tsv = handle_request("POST", "/", b"SHOW TABLES", &config);
    assert_eq!(tsv.status, "200 OK");
    assert_eq!(tsv.body, b"name\nevents\nlogs\n");

    let csv = handle_request("GET", "/?query=SHOW+TABLES&format=csv", b"", &config);
    assert_eq!(csv.status, "200 OK");
    assert_eq!(csv.body, b"name\r\nevents\r\nlogs\r\n");

    let json = handle_request("POST", "/?format=json", b"SHOW TABLES;", &config);
    assert_eq!(json.status, "200 OK");
    assert_eq!(json.body, b"[{\"name\":\"events\"},{\"name\":\"logs\"}]\n");

    let unsupported = handle_request("POST", "/", b"SHOW CREATE TABLE events", &config);
    assert_eq!(unsupported.status, "400 Bad Request");
    let explain = handle_request("POST", "/explain", b"SHOW TABLES", &config);
    assert_eq!(explain.status, "400 Bad Request");

    fs::remove_dir_all(db).unwrap();
}

#[test]
fn sql_show_tables_requires_database_mode() {
    let no_database = handle_request("POST", "/", b"SHOW TABLES", &Config::default());
    assert_eq!(no_database.status, "400 Bad Request");
}

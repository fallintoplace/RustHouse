use super::*;

#[test]
fn readiness_checks_real_source_directory_without_modification() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-readiness-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let config = Config {
        database: Some(root.clone()),
        part: None,
        max_rows: 10,
    };
    assert_eq!(
        handle_request("GET", "/ready", b"", &config).status,
        "200 OK"
    );
    assert_eq!(
        handle_request("POST", "/ready", b"", &config).status,
        "405 Method Not Allowed"
    );
    assert_eq!(
        handle_request("GET", "/ready", b"x", &config).status,
        "400 Bad Request"
    );

    std::fs::remove_dir(&root).unwrap();
    assert_eq!(
        handle_request("GET", "/ready", b"", &config).status,
        "503 Service Unavailable"
    );
    assert_eq!(
        handle_request("GET", "/ready", b"", &Config::default()).status,
        "200 OK",
    );
}

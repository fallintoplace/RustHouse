use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(statement: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

fn successful(statement: &str, db: &Path) -> String {
    let result = sql(statement, db);
    assert!(
        result.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn streams_semicolon_delimited_measurements_and_preserves_snapshot_on_error() {
    let db = std::env::temp_dir().join(format!("rusthouse-copy-ssv-{}", std::process::id()));
    let _ = fs::remove_dir_all(&db);
    fs::create_dir_all(&db).unwrap();
    successful(
        "CREATE TABLE measurements (station String, temperature Float64)",
        &db,
    );
    let input = db.join("readings.txt");
    fs::write(&input, "Berlin;12.3\nBremen;-1.1\nBerlin;3.7\n").unwrap();
    let copy = format!(
        "COPY measurements FROM '{}' WITH (FORMAT SSV)",
        input.display()
    );
    successful(&copy, &db);
    assert_eq!(
        successful(
            "SELECT station, count(*) FROM measurements GROUP BY station ORDER BY station",
            &db
        ),
        "station\tcount\nBerlin\t2\nBremen\t1\n"
    );
    assert_eq!(
        successful(
            "SELECT station, min(temperature) FROM measurements GROUP BY station ORDER BY station",
            &db
        ),
        "station\tmin(temperature)\nBerlin\t3.7\nBremen\t-1.1\n"
    );
    let previous = fs::read(db.join("measurements.parts/.rusthouse-active")).unwrap();
    fs::write(&input, "Berlin;9.2\nBremen;not-a-number\n").unwrap();
    let rejected = sql(&copy, &db);
    assert!(!rejected.status.success());
    assert_eq!(
        fs::read(db.join("measurements.parts/.rusthouse-active")).unwrap(),
        previous
    );
    assert_eq!(
        successful("SELECT count(*) FROM measurements", &db),
        "count\n3\n"
    );
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn respects_ssv_headers_and_tsv_escapes() {
    let db = std::env::temp_dir().join(format!("rusthouse-copy-ssv-header-{}", std::process::id()));
    let _ = fs::remove_dir_all(&db);
    fs::create_dir_all(&db).unwrap();
    successful(
        "CREATE TABLE readings (station String, temperature Float64)",
        &db,
    );
    let input = db.join("readings.ssv");
    fs::write(&input, "temperature;station\n12.5;North\\x3bEast\n").unwrap();
    successful(
        &format!(
            "COPY readings FROM '{}' WITH (FORMAT SSV, HEADER TRUE)",
            input.display()
        ),
        &db,
    );
    assert_eq!(
        successful("SELECT station FROM readings", &db),
        "station\nNorth;East\n"
    );
    fs::remove_dir_all(db).unwrap();
}

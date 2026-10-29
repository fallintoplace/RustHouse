use std::fs;
use std::path::Path;
use std::process::Command;

fn run(db: &Path, statement: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{statement}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn full_outer_streams_right_fact_beyond_bounded_hash_build() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-full-right-stream-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    run(&db, "CREATE TABLE dims (id UInt64)");
    run(&db, "CREATE TABLE facts (id UInt64)");
    run(&db, "INSERT INTO dims VALUES (5), (400000)");
    // Two parts make the fact larger than the 262144-row hash-build cap.
    for _ in 0..2 {
        run(&db, "INSERT INTO facts (id) SELECT number FROM numbers(131073)");
    }
    assert_eq!(
        run(&db, "SELECT d.id, f.id FROM dims d FULL JOIN facts f ON d.id = f.id WHERE f.id IS NULL LIMIT 2"),
        b"d.id\tf.id\n400000\t\\N\n"
    );
    assert_eq!(
        run(&db, "SELECT * FROM dims d FULL JOIN facts f USING (id) WHERE d.id IS NULL LIMIT 2"),
        b"d.id\n0\n0\n"
    );
    fs::remove_dir_all(db).unwrap();
}

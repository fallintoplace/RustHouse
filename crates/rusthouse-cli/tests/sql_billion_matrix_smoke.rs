use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

fn scratch() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-billion-matrix-smoke-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn sql(query: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn physical_billion_matrix_cases_match_small_immutable_parts() {
    let root = scratch();
    let database = root.join("db");
    let created = sql("CREATE TABLE events (id UInt64)", &database);
    assert!(created.status.success(), "{created:?}");

    let fixture = root.join("ids.tsv");
    let mut rows = String::new();
    for id in 0..4096 {
        rows.push_str(&format!("{id}\n"));
    }
    fs::write(&fixture, rows).unwrap();
    let copy = sql(
        &format!("COPY events FROM '{}' WITH (FORMAT TSV)", fixture.display()),
        &database,
    );
    assert!(copy.status.success(), "{copy:?}");

    let parts = database.join("events.parts");
    let generation = fs::read_to_string(parts.join(".rusthouse-active")).unwrap();
    let snapshot = parts.join(".rusthouse-versions").join(generation.trim());
    assert!(snapshot.is_dir());
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    let checked = Command::new("python3")
        .args(["-m", "bench.scale.billion_matrix", "--binary"])
        .arg(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("--part")
        .arg(&snapshot)
        .args(["--rows", "4096", "--runs", "1", "--warmups", "0", "--timeout", "120"])
        .current_dir(repo_root)
        .output()
        .unwrap();
    assert!(
        checked.status.success(),
        "physical query matrix failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    fs::remove_dir_all(root).unwrap();
}

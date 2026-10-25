use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

fn sample_part() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-sql-cli-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("count.txt"), "10\n").unwrap();
    fs::write(
        dir.join("columns.txt"),
        "columns format version: 1\n1 columns:\n\u{0060}id\u{0060} UInt64\n",
    )
    .unwrap();
    let block = include_bytes!("../../rusthouse-storage/tests/fixtures/blocks/lz4-none.bin");
    fs::write(dir.join("id.bin"), &block[..36]).unwrap();
    let mut mark = Vec::new();
    mark.extend_from_slice(&0u64.to_le_bytes());
    mark.extend_from_slice(&0u64.to_le_bytes());
    mark.extend_from_slice(&10u64.to_le_bytes());
    fs::write(dir.join("id.mrk2"), mark).unwrap();
    dir
}

fn sql(query: &str, part: Option<&Path>) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rusthouse"));
    cmd.arg("sql").arg(query);
    if let Some(path) = part {
        cmd.arg("--part").arg(path);
    }
    cmd.output().unwrap()
}

#[test]
fn executes_scalar_select_without_a_server() {
    let output = sql("SELECT 1;", None);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "1\n1\n");
}

#[test]
fn executes_generated_numbers_without_a_part() {
    let output = sql(
        "SELECT number FROM numbers(5) WHERE number >= 2 LIMIT 2",
        None,
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "number\n2\n3\n");
}

#[test]
fn sorts_generated_numbers_from_the_cli() {
    let output = sql(
        "SELECT number FROM numbers(10) ORDER BY number DESC LIMIT 3",
        None,
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "number\n9\n8\n7\n"
    );
}

#[test]
fn pages_sorted_results_from_the_cli() {
    let result = sql(
        "SELECT number FROM numbers(10) ORDER BY number DESC LIMIT 3 OFFSET 2",
        None,
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "number\n7\n6\n5\n"
    );

    let dir = sample_part();
    let result = sql("SELECT id FROM part LIMIT 2 OFFSET 3", Some(&dir));
    assert!(result.status.success(), "{result:?}");
    let id = u64::from_le_bytes(*b"aaaaaaaa");
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        format!("id\n{id}\n{id}\n")
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn calculates_aggregates_from_the_cli() {
    let output = sql(
        "SELECT sum(number) FROM numbers(1000) WHERE number >= 900",
        None,
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "sum(number)\n94950\n"
    );

    let dir = sample_part();
    let output = sql("SELECT sum(id) FROM part", Some(&dir));
    assert!(output.status.success(), "{output:?}");
    let value = u128::from(u64::from_le_bytes(*b"aaaaaaaa"));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("sum(id)\n{}\n", value * 10)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn executes_count_projection_filter_and_limit_on_real_checked_blocks() {
    let dir = sample_part();
    let value = u64::from_le_bytes(*b"aaaaaaaa");

    let count = sql("SELECT count(*) FROM part", Some(&dir));
    assert!(count.status.success(), "{count:?}");
    assert_eq!(String::from_utf8(count.stdout).unwrap(), "count\n10\n");

    let select = sql("SELECT id FROM part LIMIT 2", Some(&dir));
    assert!(select.status.success(), "{select:?}");
    assert_eq!(
        String::from_utf8(select.stdout).unwrap(),
        format!("id\n{value}\n{value}\n")
    );

    let duplicate = sql("SELECT id, id FROM part LIMIT 2", Some(&dir));
    assert!(duplicate.status.success(), "{duplicate:?}");
    assert_eq!(
        String::from_utf8(duplicate.stdout).unwrap(),
        format!("id\tid\n{value}\t{value}\n{value}\t{value}\n")
    );

    let filtered = sql(
        &format!("SELECT id FROM part WHERE id = {value} LIMIT 1"),
        Some(&dir),
    );
    assert!(filtered.status.success(), "{filtered:?}");
    assert_eq!(
        String::from_utf8(filtered.stdout).unwrap(),
        format!("id\n{value}\n")
    );

    let missing = sql("SELECT id FROM part WHERE id < 1", Some(&dir));
    assert!(missing.status.success(), "{missing:?}");
    assert_eq!(String::from_utf8(missing.stdout).unwrap(), "id\n");

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn executes_uint128_sql_with_exact_large_integer_predicates() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-sql-128-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("count.txt"), "5\n").unwrap();
    fs::write(
        dir.join("columns.txt"),
        "columns format version: 1\n1 columns:\n\u{0060}large\u{0060} UInt128\n",
    )
    .unwrap();
    let block = include_bytes!("../../rusthouse-storage/tests/fixtures/blocks/lz4-none.bin");
    fs::write(dir.join("large.bin"), &block[..36]).unwrap();
    let mut mark = Vec::new();
    mark.extend_from_slice(&0u64.to_le_bytes());
    mark.extend_from_slice(&0u64.to_le_bytes());
    mark.extend_from_slice(&5u64.to_le_bytes());
    fs::write(dir.join("large.mrk2"), mark).unwrap();

    let value = u128::from_le_bytes(*b"aaaaaaaaaaaaaaaa");
    let filtered = sql(
        &format!("SELECT large FROM part WHERE large = {value} LIMIT 2"),
        Some(&dir),
    );
    assert!(filtered.status.success(), "{filtered:?}");
    assert_eq!(
        String::from_utf8(filtered.stdout).unwrap(),
        format!("large\n{value}\n{value}\n")
    );

    let count = sql(
        "SELECT count(*) FROM part WHERE large > 18446744073709551615",
        Some(&dir),
    );
    assert!(count.status.success(), "{count:?}");
    assert_eq!(String::from_utf8(count.stdout).unwrap(), "count\n5\n");

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refuses_unsupported_sql_and_missing_parts() {
    let invalid = sql("DROP TABLE part", None);
    assert_eq!(invalid.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("only SELECT"));

    let missing = sql("SELECT id FROM part", None);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--part"));
}

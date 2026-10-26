use std::process::Command;

fn run(sql: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", sql, "--format", "tsv-no-header"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    output.stdout
}

#[test]
fn headerless_tsv_emits_only_numeric_rows() {
    assert_eq!(run("SELECT number FROM numbers(3)"), b"0\n1\n2\n");
    assert_eq!(run("SELECT number FROM numbers(0)"), b"");
    assert_eq!(run("SELECT number FROM numbers(3) LIMIT 0"), b"");
}

#[test]
fn headerless_tsv_works_for_scalar_and_aggregate_rows() {
    assert_eq!(run("SELECT 1"), b"1\n");
    assert_eq!(run("SELECT count(*) FROM numbers(5)"), b"5\n");
    assert_eq!(run("SELECT sum(number) FROM numbers(0)"), b"NULL\n");
}

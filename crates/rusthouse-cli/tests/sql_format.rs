use std::process::Command;

#[test]
fn explicit_tsv_matches_the_default_output() {
    let command = ["sql", "SELECT number FROM numbers(3)"];
    let default = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(command)
        .output()
        .unwrap();
    let explicit = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(command)
        .args(["--format", "tsv"])
        .output()
        .unwrap();
    assert!(default.status.success());
    assert!(explicit.status.success());
    assert_eq!(default.stdout, explicit.stdout);
    assert_eq!(explicit.stdout, b"number\n0\n1\n2\n");
}

#[test]
fn duplicate_and_missing_formats_are_rejected() {
    for args in [
        vec!["sql", "SELECT 1", "--format", "tsv", "--format", "csv"],
        vec!["sql", "SELECT 1", "--format"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(&args)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{args:?}");
    }
}

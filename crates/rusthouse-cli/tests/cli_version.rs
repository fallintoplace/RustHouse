use std::process::Command;

#[test]
fn long_and_short_version_flags_print_package_version() {
    let expected = format!("rusthouse {}\n", env!("CARGO_PKG_VERSION"));
    for flag in ["--version", "-V"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .arg(flag)
            .output()
            .unwrap();
        assert!(output.status.success(), "{flag}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        assert!(output.stderr.is_empty(), "{flag}");
    }
}

#[test]
fn version_rejects_unexpected_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["--version", "--help"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

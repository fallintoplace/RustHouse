use std::process::Command;

#[test]
fn verify_file_sizes_supports_hash_verification_modes() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-full-hashes-cli-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let rows = (0..2_500)
        .map(|i| vec![Some((i % 251).to_string())])
        .collect::<Vec<_>>();
    rusthouse_storage::write_wide_part(
        &part,
        &[rusthouse_storage::Column {
            name: "id".into(),
            data_type: "UInt8".into(),
        }],
        &rows,
    )
    .unwrap();

    for (option, status) in [
        (None, "file hashes not verified"),
        (Some("--hashes"), "stored hashes verified"),
        (
            Some("--full-hashes"),
            "stored and uncompressed hashes verified",
        ),
    ] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rusthouse"));
        cmd.arg("verify-file-sizes").arg(&part);
        if let Some(option) = option {
            cmd.arg(option);
        }
        let output = cmd.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8(output.stdout).unwrap().contains(status));
    }
    let invalid = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["verify-file-sizes", "missing", "--unknown"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    std::fs::remove_dir_all(part).unwrap();
}

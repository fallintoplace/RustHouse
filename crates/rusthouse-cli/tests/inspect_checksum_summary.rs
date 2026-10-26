use std::process::Command;

fn varuint(mut value: u64, bytes: &mut Vec<u8>) {
    while value >= 128 {
        bytes.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
}

#[test]
fn summarizes_manifest_bytes_without_opening_data_files() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-checksum-summary-{}.txt",
        std::process::id()
    ));
    let mut data = b"checksums format version: 3\n".to_vec();
    varuint(2, &mut data);
    for (name, size, original) in [("id.bin", 80u64, Some(240u64)), ("id.mrk2", 20u64, None)] {
        varuint(name.len() as u64, &mut data);
        data.extend_from_slice(name.as_bytes());
        varuint(size, &mut data);
        data.extend_from_slice(&0u128.to_le_bytes());
        if let Some(original) = original {
            data.push(1);
            varuint(original, &mut data);
            data.extend_from_slice(&0u128.to_le_bytes());
        } else {
            data.push(0);
        }
    }
    std::fs::write(&path, data).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-checksums")
        .arg(&path)
        .arg("--summary")
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();

    assert!(result.status.success(), "{result:?}");
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("Files: 2"));
    assert!(text.contains("Stored bytes: 100"));
    assert!(text.contains("Compressed stored bytes: 80"));
    assert!(text.contains("Uncompressed bytes (compressed files only): 240"));
    assert!(text.contains("not verified"));
}

#[test]
fn rejects_unsupported_summary_options() {
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["inspect-checksums", "missing.txt", "--json"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
}

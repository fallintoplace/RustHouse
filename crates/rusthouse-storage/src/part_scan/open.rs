use super::*;

// Path::exists follows symlinks and converts permission errors into false.
// A broken link or inaccessible competing marks file must not be ignored.
fn present_regular_marks_file(path: &Path) -> Result<bool, PartError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(PartError::InvalidScan(format!(
            "marks path is not a regular file: {}", path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(PartError::Io { path: path.to_path_buf(), source }),
    }
}

pub(super) fn numeric_marks_path(part: &Path, name: &str) -> Result<PathBuf, PartError> {
    let plain_marks = part.join(format!("{name}.mrk2"));
    let compressed_marks = part.join(format!("{name}.cmrk2"));
    let plain = present_regular_marks_file(&plain_marks)?;
    let compressed = present_regular_marks_file(&compressed_marks)?;
    match (plain, compressed) {
        (true, true) => Err(PartError::InvalidScan(format!(
            "multiple mark files for {name}"
        ))),
        (_, true) => Ok(compressed_marks),
        _ => Ok(plain_marks),
    }
}

pub(super) fn open_numeric_stream(
    part: &Path,
    name: &str,
    data_type: &str,
) -> Result<ColumnStream, PartError> {
    let marks_path = numeric_marks_path(part, name)?;
    let marks_file = read_wide_marks(&marks_path)?;
    let data_path = part.join(format!("{name}.bin"));
    let mut reader = CompressedBlockReader::open(&data_path)?;
    crate::mark_validation::validate_wide_marks_reader(&marks_file, &mut reader)?;
    // Mark validation leaves the reader at EOF. Rewind the verified
    // file instead of reopening it for the actual granule scan.
    reader.seek_to_block(0)?;
    Ok(ColumnStream {
        data_type: data_type.to_owned(),
        marks: marks_file.marks,
        reader,
        cached: None,
    })
}

#[cfg(test)]
mod mark_path_validation_tests {
    use super::*;

    #[test]
    fn chooses_one_regular_marks_file_and_refuses_ambiguous_paths() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-mark-selection-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let plain = dir.join("id.mrk2");
        let compressed = dir.join("id.cmrk2");
        assert_eq!(numeric_marks_path(&dir, "id").unwrap(), plain);
        std::fs::write(&compressed, b"").unwrap();
        assert_eq!(numeric_marks_path(&dir, "id").unwrap(), compressed);
        std::fs::write(&plain, b"").unwrap();
        assert!(numeric_marks_path(&dir, "id").is_err());
        std::fs::remove_file(&plain).unwrap();
        std::fs::remove_file(&compressed).unwrap();
        std::fs::create_dir(&compressed).unwrap();
        assert!(numeric_marks_path(&dir, "id").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn does_not_silently_ignore_a_broken_competing_symlink() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-mark-broken-link-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("id.mrk2"), b"").unwrap();
        symlink("nonexistent", dir.join("id.cmrk2")).unwrap();
        assert!(numeric_marks_path(&dir, "id").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

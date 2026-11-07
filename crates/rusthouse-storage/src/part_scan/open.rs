use super::*;

use crate::marked_stream::{marks_path as numeric_marks_path, open_validated_column};

pub(super) fn open_numeric_stream(
    part: &Path,
    name: &str,
    data_type: &str,
) -> Result<ColumnStream, PartError> {
    let (marks, reader) = open_validated_column(part, name)?;
    Ok(ColumnStream {
        data_type: data_type.to_owned(),
        marks,
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

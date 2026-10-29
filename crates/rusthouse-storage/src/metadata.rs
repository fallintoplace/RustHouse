use crate::{PartError, parse_columns};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_COUNT_BYTES: u64 = 64;
const MAX_COLUMNS_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartMetadata {
    pub path: PathBuf,
    pub rows: u64,
    pub columns: Vec<Column>,
}

fn io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn invalid_metadata(path: &Path, reason: &'static str) -> PartError {
    io_error(path, io::Error::new(io::ErrorKind::InvalidData, reason))
}

fn validate_part_directory(path: &Path) -> Result<(), PartError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err(invalid_metadata(path, "part path must be a real directory"));
        }
        Ok(_) => {}
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error(path, source)),
    }
    Ok(())
}

/// Read only the bounded row-count file for a previously validated part.
/// OFFSET skipping does not need to reparse the complete columns.txt schema.
pub fn read_part_row_count(path: impl AsRef<Path>) -> Result<u64, PartError> {
    let path = path.as_ref();
    validate_part_directory(path)?;
    parse_count(&read_metadata(&path.join("count.txt"), MAX_COUNT_BYTES)?)
}

pub fn inspect_part(path: impl AsRef<Path>) -> Result<PartMetadata, PartError> {
    let path = path.as_ref();
    let rows = read_part_row_count(path)?;
    let columns = parse_columns(&read_metadata(
        &path.join("columns.txt"),
        MAX_COLUMNS_BYTES,
    )?)?;

    Ok(PartMetadata {
        path: path.to_path_buf(),
        rows,
        columns,
    })
}

fn ensure_metadata_size(path: &Path, size: u64, max_bytes: u64) -> Result<(), PartError> {
    if size > max_bytes {
        return Err(invalid_metadata(path, "metadata exceeds size limit"));
    }
    Ok(())
}

fn read_bounded_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, PartError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.file_type().is_file() {
        return Err(invalid_metadata(path, "metadata must be a regular file"));
    }
    ensure_metadata_size(path, metadata.len(), max_bytes)?;
    let file = fs::File::open(path).map_err(|source| io_error(path, source))?;
    let opened = file.metadata().map_err(|source| io_error(path, source))?;
    if !opened.file_type().is_file() || opened.len() > max_bytes {
        return Err(invalid_metadata(path, "metadata changed before read"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() {
            return Err(invalid_metadata(path, "metadata file replaced before read"));
        }
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    ensure_metadata_size(path, bytes.len() as u64, max_bytes)?;
    Ok(bytes)
}

fn read_metadata(path: &Path, max_bytes: u64) -> Result<String, PartError> {
    String::from_utf8(read_bounded_file(path, max_bytes)?)
        .map_err(|source| io_error(path, io::Error::new(io::ErrorKind::InvalidData, source)))
}

fn parse_count(content: &str) -> Result<u64, PartError> {
    let digits = content
        .strip_suffix("\r\n")
        .or_else(|| content.strip_suffix('\n'))
        .unwrap_or(content);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PartError::InvalidCount(content.to_owned()));
    }
    digits
        .parse()
        .map_err(|_| PartError::InvalidCount(content.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_part_root() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "rusthouse-metadata-part-root-{}",
            std::process::id()
        ));
        let link = root.with_extension("link");
        fs::create_dir(&root).unwrap();
        symlink(&root, &link).unwrap();
        assert!(matches!(
            inspect_part(&link),
            Err(PartError::Io { source, .. }) if source.kind() == io::ErrorKind::InvalidData
        ));
        fs::remove_file(link).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_metadata() {
        use std::os::unix::fs::symlink;

        let dir =
            std::env::temp_dir().join(format!("rusthouse-metadata-symlink-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("real-count.txt"), b"42\n").unwrap();
        symlink("real-count.txt", dir.join("count.txt")).unwrap();
        assert!(matches!(
            read_metadata(&dir.join("count.txt"), MAX_COUNT_BYTES),
            Err(PartError::Io { source, .. }) if source.kind() == io::ErrorKind::InvalidData
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn inspects_fixture_part_without_writing() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/all_1_1_0");
        let metadata = inspect_part(&path).unwrap();
        assert_eq!(metadata.path, path);
        assert_eq!(metadata.rows, 42);
        assert_eq!(read_part_row_count(&path).unwrap(), 42);
        assert_eq!(metadata.columns.len(), 2);
        assert_eq!(metadata.columns[0].name, "id");
        assert_eq!(metadata.columns[1].data_type, "LowCardinality(String)");
    }

    #[test]
    fn reads_part_counts_without_parsing_a_wide_schema() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-count-only-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("count.txt"), "4096\n").unwrap();
        fs::write(root.join("columns.txt"), "not a valid schema\n").unwrap();
        assert_eq!(read_part_row_count(&root).unwrap(), 4096);
        assert!(inspect_part(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_missing_count_file() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/missing");
        assert!(matches!(inspect_part(path), Err(PartError::Io { .. })));
    }

    #[test]
    fn accepts_plain_and_newline_terminated_counts() {
        assert_eq!(parse_count("42").unwrap(), 42);
        assert_eq!(parse_count("42\n").unwrap(), 42);
        assert_eq!(parse_count("42\r\n").unwrap(), 42);
        assert_eq!(parse_count("18446744073709551615\n").unwrap(), u64::MAX);
    }

    #[test]
    fn limits_count_and_column_metadata_before_parsing() {
        let dir =
            std::env::temp_dir().join(format!("rusthouse-bounded-metadata-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n0 columns:\n",
        )
        .unwrap();
        fs::write(
            dir.join("count.txt"),
            vec![b'1'; MAX_COUNT_BYTES as usize + 1],
        )
        .unwrap();
        assert!(matches!(
            inspect_part(&dir),
            Err(PartError::Io { path, source })
                if path.ends_with("count.txt")
                    && source.kind() == io::ErrorKind::InvalidData
        ));

        fs::write(dir.join("count.txt"), "0\n").unwrap();
        let file = fs::File::create(dir.join("columns.txt")).unwrap();
        file.set_len(MAX_COLUMNS_BYTES + 1).unwrap();
        assert!(matches!(
            inspect_part(&dir),
            Err(PartError::Io { path, source })
                if path.ends_with("columns.txt")
                    && source.kind() == io::ErrorKind::InvalidData
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_invalid_utf8_metadata() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-invalid-utf8-metadata-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("count.txt"), "1\n").unwrap();
        fs::write(dir.join("columns.txt"), [0xff, 0xfe]).unwrap();
        assert!(matches!(
            inspect_part(&dir),
            Err(PartError::Io { source, .. })
                if source.kind() == io::ErrorKind::InvalidData
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_malformed_counts() {
        for input in [
            "",
            "\n",
            " 42\n",
            "42 \n",
            "42\n\n",
            "42\r\n\n",
            "42\r",
            "-1\n",
            "+42\n",
            "+0",
            "18446744073709551616\n",
        ] {
            assert!(
                matches!(parse_count(input), Err(PartError::InvalidCount(_))),
                "{input:?}"
            );
        }
    }
}

#[cfg(all(test, unix))]
mod metadata_identity_tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn same_size_file_substitution_changes_inode_identity() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-metadata-identity-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let first = root.join("first");
        let second = root.join("second");
        fs::write(&first, "123").unwrap();
        fs::write(&second, "456").unwrap();
        let original = fs::symlink_metadata(&first).unwrap();
        let replacement = fs::File::open(&second).unwrap().metadata().unwrap();
        assert_eq!(original.len(), replacement.len());
        assert_ne!(original.ino(), replacement.ino());
        fs::remove_dir_all(root).unwrap();
    }
}

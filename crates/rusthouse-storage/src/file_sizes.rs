use crate::{PartError, read_checksums};
use std::fs;
use std::io;
use std::path::Path;

/// Counts manifest entries whose stored sizes match the actual local files.
/// File hashes are not checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSizeVerification {
    pub files: usize,
    pub total_bytes: u64,
}

/// Check that every file listed by checksums.txt exists with the expected size.
/// Symlinks are not followed, including symlinks in nested paths.
pub fn verify_part_file_sizes(part: impl AsRef<Path>) -> Result<FileSizeVerification, PartError> {
    let part = part.as_ref();
    let root_metadata = fs::symlink_metadata(part).map_err(|source| io_error(part, source))?;
    if !root_metadata.file_type().is_dir() {
        return Err(invalid(part, "part path is not a directory"));
    }
    let checksums_path = part.join("checksums.txt");
    let metadata = fs::symlink_metadata(&checksums_path)
        .map_err(|source| io_error(&checksums_path, source))?;
    ensure_regular_file(
        &metadata,
        &checksums_path,
        "checksums.txt is not a regular file",
    )?;
    let manifest = read_checksums(&checksums_path)?;
    let mut total_bytes = 0u64;

    for entry in &manifest.files {
        verify_listed_file(part, &manifest.path, &entry.name, entry.size)?;
        total_bytes = total_bytes
            .checked_add(entry.size)
            .ok_or_else(|| invalid(&manifest.path, "file sizes overflow u64"))?;
    }

    Ok(FileSizeVerification {
        files: manifest.files.len(),
        total_bytes,
    })
}

fn ensure_regular_file(
    metadata: &fs::Metadata,
    error_path: &Path,
    reason: &'static str,
) -> Result<(), PartError> {
    if !metadata.file_type().is_file() {
        return Err(invalid(error_path, reason));
    }
    Ok(())
}

fn verify_listed_file(
    part: &Path,
    manifest_path: &Path,
    name: &str,
    expected_size: u64,
) -> Result<(), PartError> {
    let mut path = part.to_path_buf();
    let mut components = name.split('/').peekable();
    while let Some(component) = components.next() {
        path.push(component);
        verify_component(
            &path,
            manifest_path,
            components.peek().is_some(),
            expected_size,
        )?;
    }
    Ok(())
}

fn verify_component(
    path: &Path,
    manifest_path: &Path,
    is_parent: bool,
    expected_size: u64,
) -> Result<(), PartError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if is_parent {
        if !metadata.file_type().is_dir() {
            return Err(invalid(
                manifest_path,
                "listed file parent is not a directory",
            ));
        }
    } else {
        verify_leaf_size(&metadata, manifest_path, expected_size)?;
    }
    Ok(())
}

fn verify_leaf_size(
    metadata: &fs::Metadata,
    manifest_path: &Path,
    expected_size: u64,
) -> Result<(), PartError> {
    ensure_regular_file(
        metadata,
        manifest_path,
        "listed entry is not a regular file",
    )?;
    if metadata.len() != expected_size {
        return Err(invalid(
            manifest_path,
            "file size differs from checksums.txt",
        ));
    }
    Ok(())
}

fn invalid(path: &Path, reason: &'static str) -> PartError {
    PartError::InvalidChecksums {
        path: path.to_path_buf(),
        reason,
    }
}

fn io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-checksum-sizes-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn varuint(mut value: u64, bytes: &mut Vec<u8>) {
        while value >= 128 {
            bytes.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        bytes.push(value as u8);
    }

    fn manifest(entries: &[(&str, u64)]) -> Vec<u8> {
        let mut bytes = b"checksums format version: 3\n".to_vec();
        varuint(entries.len() as u64, &mut bytes);
        for (name, size) in entries {
            varuint(name.len() as u64, &mut bytes);
            bytes.extend_from_slice(name.as_bytes());
            varuint(*size, &mut bytes);
            bytes.extend_from_slice(&0u128.to_le_bytes());
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn leaf_validation_checks_regular_files_before_sizes() {
        let dir = temp_dir();
        let file = dir.join("a.bin");
        fs::write(&file, [1u8, 2, 3]).unwrap();
        let metadata = fs::symlink_metadata(&file).unwrap();
        assert!(verify_leaf_size(&metadata, &file, 3).is_ok());
        assert!(matches!(
            verify_leaf_size(&metadata, &file, 4),
            Err(PartError::InvalidChecksums {
                reason: "file size differs from checksums.txt",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn verifies_nested_files_and_reports_total_size() {
        let dir = temp_dir();
        fs::write(dir.join("id.bin"), [1, 2, 3]).unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/marks.mrk2"), [0; 8]).unwrap();
        fs::write(
            dir.join("checksums.txt"),
            manifest(&[("id.bin", 3), ("sub/marks.mrk2", 8)]),
        )
        .unwrap();

        assert_eq!(
            verify_part_file_sizes(&dir).unwrap(),
            FileSizeVerification {
                files: 2,
                total_bytes: 11
            }
        );
        // Size validation is not a hash check.
        fs::write(dir.join("id.bin"), [9, 8, 7]).unwrap();
        assert!(verify_part_file_sizes(&dir).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_missing_files_and_size_mismatches() {
        let dir = temp_dir();
        fs::write(dir.join("checksums.txt"), manifest(&[("a.bin", 4)])).unwrap();
        assert!(matches!(
            verify_part_file_sizes(&dir),
            Err(PartError::Io { path, source })
                if path.ends_with("a.bin") && source.kind() == io::ErrorKind::NotFound
        ));

        fs::write(dir.join("a.bin"), [1u8, 2, 3]).unwrap();
        assert!(matches!(
            verify_part_file_sizes(&dir),
            Err(PartError::InvalidChecksums {
                reason: "file size differs from checksums.txt",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refuses_directory_entries() {
        let dir = temp_dir();
        fs::create_dir(dir.join("not-a-file")).unwrap();
        fs::write(dir.join("checksums.txt"), manifest(&[("not-a-file", 0)])).unwrap();
        assert!(matches!(
            verify_part_file_sizes(&dir),
            Err(PartError::InvalidChecksums {
                reason: "listed entry is not a regular file",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_files_and_directories() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir();
        fs::create_dir(dir.join("real")).unwrap();
        fs::write(dir.join("real/file.bin"), [1u8]).unwrap();
        symlink("real/file.bin", dir.join("file.bin")).unwrap();
        symlink("real", dir.join("link")).unwrap();
        fs::write(dir.join("checksums.txt"), manifest(&[("file.bin", 1)])).unwrap();
        assert!(matches!(
            verify_part_file_sizes(&dir),
            Err(PartError::InvalidChecksums {
                reason: "listed entry is not a regular file",
                ..
            })
        ));

        fs::write(dir.join("checksums.txt"), manifest(&[("link/file.bin", 1)])).unwrap();
        assert!(matches!(
            verify_part_file_sizes(&dir),
            Err(PartError::InvalidChecksums {
                reason: "listed file parent is not a directory",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_checksums_manifest() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir();
        let target = dir.join("actual-checksums.txt");
        fs::write(&target, manifest(&[])).unwrap();
        symlink(&target, dir.join("checksums.txt")).unwrap();

        assert!(matches!(
            verify_part_file_sizes(&dir),
            Err(PartError::InvalidChecksums {
                reason: "checksums.txt is not a regular file",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_regular_files_used_as_parent_directories() {
        let dir = temp_dir();
        fs::write(dir.join("parent"), [1u8]).unwrap();
        fs::write(
            dir.join("checksums.txt"),
            manifest(&[("parent/child.bin", 0)]),
        )
        .unwrap();

        assert!(matches!(
            verify_part_file_sizes(&dir),
            Err(PartError::InvalidChecksums {
                reason: "listed file parent is not a directory",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_part_roots() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir();
        fs::write(dir.join("checksums.txt"), manifest(&[])).unwrap();
        let alias = dir.with_extension("link");
        symlink(&dir, &alias).unwrap();

        assert!(matches!(
            verify_part_file_sizes(&alias),
            Err(PartError::InvalidChecksums {
                reason: "part path is not a directory",
                ..
            })
        ));
        fs::remove_file(alias).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn supports_empty_manifest() {
        let dir = temp_dir();
        fs::write(dir.join("checksums.txt"), manifest(&[])).unwrap();
        assert_eq!(
            verify_part_file_sizes(&dir).unwrap(),
            FileSizeVerification {
                files: 0,
                total_bytes: 0
            }
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

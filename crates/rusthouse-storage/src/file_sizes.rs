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
    let manifest = read_checksums(part.join("checksums.txt"))?;
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
        let metadata = fs::symlink_metadata(&path).map_err(|source| io_error(&path, source))?;
        if components.peek().is_some() {
            if !metadata.file_type().is_dir() {
                return Err(invalid(
                    manifest_path,
                    "listed file parent is not a directory",
                ));
            }
        } else {
            if !metadata.file_type().is_file() {
                return Err(invalid(manifest_path, "listed entry is not a regular file"));
            }
            if metadata.len() != expected_size {
                return Err(invalid(manifest_path, "file size differs from checksums.txt"));
            }
        }
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

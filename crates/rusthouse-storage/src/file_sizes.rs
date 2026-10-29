use crate::{CompressedBlockReader, PartError, UncompressedChecksum, read_checksums};
use naive_cityhash::{U128, cityhash128_with_seed};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const BLOCK_BYTES: usize = 2048;

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
    verify_part_files(part.as_ref(), false, false)
}

/// Verify stored-byte CityHash128WithSeed checksums, in addition to file sizes.
///
/// This intentionally checks the stored hash only, not the optional
/// uncompressed hash of compressed streams. It reads each listed file in
/// bounded 2048-byte chunks, matching ClickHouse's HashingWriteBuffer.
/// Run only on frozen parts while writers are stopped.
pub fn verify_part_file_hashes(part: impl AsRef<Path>) -> Result<FileSizeVerification, PartError> {
    verify_part_files(part.as_ref(), true, false)
}

/// Verify stored and decompressed CityHash128WithSeed hashes from checksums.txt.
/// Compressed streams are decoded block by block using bounded memory.
pub fn verify_part_file_hashes_full(
    part: impl AsRef<Path>,
) -> Result<FileSizeVerification, PartError> {
    verify_part_files(part.as_ref(), true, true)
}

fn verify_part_files(
    part: &Path,
    full_hashes: bool,
    verify_uncompressed: bool,
) -> Result<FileSizeVerification, PartError> {
    let manifest = read_checksums(&validated_checksums_path(part)?)?;
    let mut total_bytes = 0u64;

    for entry in &manifest.files {
        verify_listed_file(part, &manifest.path, &entry.name, entry.size)?;
        if full_hashes {
            verify_stored_hash(&part.join(&entry.name), entry.size, entry.hash)?;
        }
        if let (true, Some(uncompressed)) = (verify_uncompressed, &entry.uncompressed) {
            verify_uncompressed_hash(&part.join(&entry.name), uncompressed)?;
        }
        total_bytes = total_bytes
            .checked_add(entry.size)
            .ok_or_else(|| invalid(&manifest.path, "file sizes overflow u64"))?;
    }

    Ok(FileSizeVerification {
        files: manifest.files.len(),
        total_bytes,
    })
}

fn hash_stored_file(path: &Path, expected_size: u64) -> Result<u128, PartError> {
    let mut file = fs::File::open(path).map_err(|source| io_error(path, source))?;
    #[cfg(unix)]
    ensure_opened_file_identity(path, &file)?;
    let mut hash = U128::new(0, 0);
    let mut remaining = expected_size;
    let mut buffer = [0u8; BLOCK_BYTES];
    while remaining > 0 {
        let length = remaining.min(BLOCK_BYTES as u64) as usize;
        file.read_exact(&mut buffer[..length])
            .map_err(|source| io_error(path, source))?;
        hash = cityhash128_with_seed(&buffer[..length], hash);
        remaining -= length as u64;
    }
    // Catch files extended after the size check, as well as changed data.
    if file
        .read(&mut buffer[..1])
        .map_err(|source| io_error(path, source))?
        != 0
    {
        return Err(invalid(path, "file changed during hash verification"));
    }
    #[cfg(unix)]
    ensure_opened_file_identity(path, &file)?;
    Ok(u128::from(hash))
}

fn verify_stored_hash(
    path: &Path,
    expected_size: u64,
    expected_hash: u128,
) -> Result<(), PartError> {
    if hash_stored_file(path, expected_size)? != expected_hash {
        return Err(invalid(path, "stored file hash differs from checksums.txt"));
    }
    Ok(())
}

/// Hash the logical byte stream, not the individual compressed block boundaries.
/// ClickHouse's HashingWriteBuffer uses a continuous 2048-byte chunk sequence.
fn verify_uncompressed_hash(path: &Path, expected: &UncompressedChecksum) -> Result<(), PartError> {
    let mut reader = CompressedBlockReader::open(path)?;
    let mut hash = U128::new(0, 0);
    let mut chunk = [0u8; BLOCK_BYTES];
    let mut filled = 0usize;
    let mut total_bytes = 0u64;
    while let Some(block) = reader.next_decoded() {
        let data = block?.data;
        total_bytes = total_bytes
            .checked_add(data.len() as u64)
            .ok_or_else(|| invalid(path, "uncompressed file size overflows u64"))?;
        if total_bytes > expected.size {
            return Err(invalid(
                path,
                "uncompressed file size differs from checksums.txt",
            ));
        }
        let mut remaining = data.as_slice();
        while !remaining.is_empty() {
            let take = (BLOCK_BYTES - filled).min(remaining.len());
            chunk[filled..filled + take].copy_from_slice(&remaining[..take]);
            filled += take;
            remaining = &remaining[take..];
            if filled == BLOCK_BYTES {
                hash = cityhash128_with_seed(&chunk, hash);
                filled = 0;
            }
        }
    }
    if total_bytes != expected.size {
        return Err(invalid(
            path,
            "uncompressed file size differs from checksums.txt",
        ));
    }
    if filled > 0 {
        hash = cityhash128_with_seed(&chunk[..filled], hash);
    }
    if u128::from(hash) != expected.hash {
        return Err(invalid(
            path,
            "uncompressed file hash differs from checksums.txt",
        ));
    }
    Ok(())
}

/// Detect inode replacement between path validation, open, and hash completion.
#[cfg(unix)]
fn ensure_opened_file_identity(path: &Path, file: &fs::File) -> Result<(), PartError> {
    use std::os::unix::fs::MetadataExt;

    let named = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    let opened = file.metadata().map_err(|source| io_error(path, source))?;
    if !named.file_type().is_file()
        || !opened.file_type().is_file()
        || named.dev() != opened.dev()
        || named.ino() != opened.ino()
    {
        return Err(invalid(
            path,
            "listed file was replaced during hash verification",
        ));
    }
    Ok(())
}

fn validated_checksums_path(part: &Path) -> Result<PathBuf, PartError> {
    verify_part_root(part)?;
    let path = part.join("checksums.txt");
    let metadata = nofollow_metadata(&path)?;
    ensure_regular_file(&metadata, &path, "checksums.txt is not a regular file")?;
    Ok(path)
}

/// Read metadata for the directory entry itself, never its symlink target.
fn nofollow_metadata(path: &Path) -> Result<fs::Metadata, PartError> {
    fs::symlink_metadata(path).map_err(|source| io_error(path, source))
}

fn verify_part_root(part: &Path) -> Result<(), PartError> {
    let root_metadata = nofollow_metadata(part)?;
    if !root_metadata.file_type().is_dir() {
        return Err(invalid(part, "part path is not a directory"));
    }
    Ok(())
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
    let metadata = nofollow_metadata(path)?;
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
    fn full_hash_check_verifies_decompressed_data_across_blocks() {
        let dir = temp_dir();
        let part = dir.join("all_1_1_0");
        let rows = (0..5_000)
            .map(|number| vec![Some((number % 251).to_string())])
            .collect::<Vec<_>>();
        crate::write_wide_part_with_compression(
            &part,
            &[crate::Column {
                name: "id".into(),
                data_type: "UInt8".into(),
            }],
            &rows,
            crate::PartCompression::Zstd,
        )
        .unwrap();

        assert_eq!(verify_part_file_hashes_full(&part).unwrap().files, 4);
        let entry = read_checksums(part.join("checksums.txt"))
            .unwrap()
            .files
            .into_iter()
            .find(|entry| entry.name == "id.bin")
            .unwrap();
        let original = entry.uncompressed.unwrap();
        let mut bad_size = original.clone();
        bad_size.size += 1;
        assert!(matches!(
            verify_uncompressed_hash(&part.join("id.bin"), &bad_size),
            Err(PartError::InvalidChecksums {
                reason: "uncompressed file size differs from checksums.txt",
                ..
            })
        ));

        let manifest = part.join("checksums.txt");
        let mut bytes = fs::read(&manifest).unwrap();
        let needle = original.hash.to_le_bytes();
        let pos = bytes
            .windows(needle.len())
            .rposition(|window| window == needle)
            .unwrap();
        bytes[pos] ^= 1;
        fs::write(&manifest, bytes).unwrap();
        assert!(verify_part_file_hashes(&part).is_ok());
        assert!(matches!(
            verify_part_file_hashes_full(&part),
            Err(PartError::InvalidChecksums {
                reason: "uncompressed file hash differs from checksums.txt",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn full_hash_check_detects_same_size_metadata_corruption() {
        let dir = temp_dir();
        let part = dir.join("all_1_1_0");
        let rows = (0..4096)
            .map(|number| vec![Some(number.to_string())])
            .collect::<Vec<_>>();
        crate::write_wide_part(
            &part,
            &[crate::Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &rows,
        )
        .unwrap();
        let verified = verify_part_file_hashes(&part).unwrap();
        assert_eq!(verified.files, 4);
        assert!(verified.total_bytes > 2048);

        let path = part.join("columns.txt");
        let mut content = fs::read(&path).unwrap();
        let offset = content.windows(4).position(|part| part == b"`id`").unwrap() + 1;
        content[offset] = b'x';
        fs::write(&path, content).unwrap();
        assert!(verify_part_file_sizes(&part).is_ok());
        assert!(matches!(
            verify_part_file_hashes(&part),
            Err(PartError::InvalidChecksums {
                reason: "stored file hash differs from checksums.txt",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_file_replacement_after_opening() {
        let dir = temp_dir();
        let path = dir.join("data.bin");
        fs::write(&path, b"same contents").unwrap();
        let file = fs::File::open(&path).unwrap();
        ensure_opened_file_identity(&path, &file).unwrap();

        fs::rename(&path, dir.join("old.bin")).unwrap();
        fs::write(&path, b"same contents").unwrap();
        assert!(matches!(
            ensure_opened_file_identity(&path, &file),
            Err(PartError::InvalidChecksums {
                reason: "listed file was replaced during hash verification",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
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

//! Enumerate immutable files that may be copied to another storage node.
use crate::{PartError, query_parts, resolve_active_snapshot};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryEntry {
    pub relative_path: PathBuf,
    pub bytes: u64,
}

fn io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io {
        path: path.to_owned(),
        source,
    }
}

fn verified_file_size(path: &Path) -> Result<u64, PartError> {
    // Do not follow links while determining the file type and size.
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if !metadata.file_type().is_file() {
        return Err(PartError::InvalidScan(format!(
            "replication inventory requires regular files: {}",
            path.display()
        )));
    }
    Ok(metadata.len())
}

/// Inventory a frozen table root. Paths are relative to its active generation.
/// Reject links and unexpected directories inside part directories.
pub fn inventory_snapshot(
    root: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<Vec<InventoryEntry>, PartError> {
    let active = resolve_active_snapshot(root)?;
    let parts = query_parts(&active)?;
    // A typical wide part has several streams plus its metadata files.
    // Cap the estimate to avoid excessive allocations for sparse inventories.
    let estimated_file_capacity = parts.len().saturating_mul(8).min(max_files).min(1024);
    let mut files = Vec::with_capacity(estimated_file_capacity);
    let mut total_bytes = 0u64;
    for part in parts {
        let name = part
            .file_name()
            .ok_or_else(|| PartError::InvalidScan("part has no directory name".into()))?;
        for entry in fs::read_dir(&part).map_err(|error| io_error(&part, error))? {
            let entry = entry.map_err(|error| io_error(&part, error))?;
            let path = entry.path();
            let bytes = verified_file_size(&path)?;
            total_bytes = total_bytes
                .checked_add(bytes)
                .ok_or_else(|| PartError::InvalidScan("inventory size overflow".into()))?;
            if files.len() >= max_files || total_bytes > max_bytes {
                return Err(PartError::InvalidScan(
                    "replication inventory exceeds configured bounds".into(),
                ));
            }
            files.push(InventoryEntry {
                relative_path: PathBuf::from(name).join(entry.file_name()),
                bytes,
            });
        }
    }
    files.sort_unstable_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

/// Stored-byte checksum of one frozen snapshot file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedInventoryEntry {
    pub relative_path: PathBuf,
    pub bytes: u64,
    pub crc32c: u32,
}

/// Build a bounded, content-checksummed inventory for replica reconciliation.
/// CRC32C is an accidental corruption check, not authentication.
fn checksum_inventory_entry(
    active: &Path,
    entry: &InventoryEntry,
) -> Result<CheckedInventoryEntry, PartError> {
    let path = active.join(&entry.relative_path);
    let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
    if !metadata.file_type().is_file() || metadata.len() != entry.bytes {
        return Err(PartError::InvalidScan(
            "snapshot inventory file changed before hashing".into(),
        ));
    }
    let mut file = fs::File::open(&path).map_err(|error| io_error(&path, error))?;
    let opened = file.metadata().map_err(|error| io_error(&path, error))?;
    if !opened.file_type().is_file() || opened.len() != entry.bytes {
        return Err(PartError::InvalidScan(
            "snapshot inventory file changed before hashing".into(),
        ));
    }
    #[cfg(unix)]
    if !same_file_identity(&metadata, &opened) {
        return Err(PartError::InvalidScan(
            "snapshot inventory file replaced before hashing".into(),
        ));
    }
    let (bytes, crc32c) = crate::distributed_crc32c::crc32c_reader(&mut file, entry.bytes)
        .map_err(|error| io_error(&path, error))?;
    if bytes != entry.bytes {
        return Err(PartError::InvalidScan(
            "snapshot inventory file changed during hashing".into(),
        ));
    }
    Ok(CheckedInventoryEntry {
        relative_path: entry.relative_path.clone(),
        bytes,
        crc32c,
    })
}

pub fn inventory_snapshot_crc32c(
    root: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<Vec<CheckedInventoryEntry>, PartError> {
    let active = resolve_active_snapshot(root)?;
    let entries = inventory_snapshot(&active, max_files, max_bytes)?;
    const PARALLEL_THRESHOLD: usize = 64;
    const MAX_HASH_WORKERS: usize = 4;
    let workers = std::thread::available_parallelism()
        .map_or(1, |parallelism| parallelism.get())
        .min(MAX_HASH_WORKERS);
    if entries.len() < PARALLEL_THRESHOLD || workers == 1 {
        return entries
            .iter()
            .map(|entry| checksum_inventory_entry(&active, entry))
            .collect();
    }

    // Hash each file independently with bounded concurrency. Keep original
    // inventory order, including deterministic first-error reporting.
    use std::sync::atomic::{AtomicUsize, Ordering};
    let cursor = AtomicUsize::new(0);
    let mut slots: Vec<Option<Result<CheckedInventoryEntry, PartError>>> =
        (0..entries.len()).map(|_| None).collect();
    std::thread::scope(|scope| -> Result<(), PartError> {
        let handles = (0..workers)
            .map(|_| {
                let cursor = &cursor;
                let entries = &entries;
                let active = &active;
                scope.spawn(move || {
                    let mut completed = Vec::new();
                    loop {
                        let index = cursor.fetch_add(1, Ordering::Relaxed);
                        if index >= entries.len() {
                            break;
                        }
                        completed.push((index, checksum_inventory_entry(active, &entries[index])));
                    }
                    completed
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            let completed = handle.join().map_err(|_| {
                PartError::InvalidScan("snapshot checksum worker panicked".into())
            })?;
            for (index, item) in completed {
                slots[index] = Some(item);
            }
        }
        Ok(())
    })?;
    slots
        .into_iter()
        .map(|slot| slot.expect("every claimed file has one result"))
        .collect()
}

#[cfg(unix)]
fn same_file_identity(path_metadata: &fs::Metadata, opened_metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    path_metadata.dev() == opened_metadata.dev()
        && path_metadata.ino() == opened_metadata.ino()
}

#[cfg(all(test, unix))]
mod immutable_identity_tests {
    use super::*;

    #[test]
    fn same_length_different_inodes_are_not_the_same_object() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-distributed_inventory-identity-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let first = root.join("one");
        let second = root.join("two");
        fs::write(&first, b"abcd").unwrap();
        fs::write(&second, b"wxyz").unwrap();
        let before = fs::symlink_metadata(&first).unwrap();
        let opened = File::open(&second).unwrap().metadata().unwrap();
        assert_eq!(before.len(), opened.len());
        assert!(!same_file_identity(&before, &opened));
        assert!(same_file_identity(&before, &File::open(&first).unwrap().metadata().unwrap()));
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn inventories_real_wide_part_with_bounded_sizes() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-inventory-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let part = root.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("7".into())]],
        )
        .unwrap();
        let files = inventory_snapshot(&root, 32, 1_000_000).unwrap();
        assert!(
            files
                .iter()
                .any(|entry| entry.relative_path == Path::new("all_0_0_0/count.txt"))
        );
        assert!(files.iter().all(|entry| entry.bytes <= 1_000_000));
        assert!(inventory_snapshot(&root, 1, 1_000_000).is_err());
        assert!(inventory_snapshot(&root, 32, 1).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_files() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "rusthouse-inventory-link-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let part = root.join("all_0_0_0");
        fs::create_dir_all(&part).unwrap();
        fs::write(part.join("count.txt"), "0\n").unwrap();
        fs::write(
            part.join("columns.txt"),
            "columns format version: 1\n1 columns:\n`id` UInt64\n",
        )
        .unwrap();
        symlink(part.join("count.txt"), part.join("shadow.txt")).unwrap();
        assert!(inventory_snapshot(&root, 32, 1_000_000).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_unexpected_directories_inside_parts() {
        let root =
            std::env::temp_dir().join(format!("rusthouse-inventory-dir-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let part = root.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("7".into())]],
        )
        .unwrap();
        fs::create_dir(part.join("unexpected")).unwrap();
        assert!(inventory_snapshot(&root, 32, 1_000_000).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod checksummed_inventory_tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn parallel_checksums_keep_sorted_paths_and_exact_file_bytes() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-parallel-crc32c-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        for part_number in 0..40 {
            let part = root.join(format!("all_{part_number}_{part_number}_0"));
            write_wide_part(
                &part,
                &[Column { name: "id".into(), data_type: "UInt64".into() }],
                &[vec![Some(part_number.to_string())]],
            ).unwrap();
        }
        let inventory = inventory_snapshot_crc32c(&root, 512, 64 * 1024 * 1024).unwrap();
        assert!(inventory.len() >= 64);
        assert!(inventory.windows(2).all(|pair| {
            pair[0].relative_path < pair[1].relative_path
        }));
        for entry in &inventory {
            let bytes = fs::read(root.join(&entry.relative_path)).unwrap();
            assert_eq!(entry.bytes, bytes.len() as u64);
            assert_eq!(entry.crc32c, crate::distributed_crc32c::crc32c(&bytes));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checksums_actual_wide_part_files() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-hashed-inventory-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let part = root.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("9".into())]],
        ).unwrap();
        let inventory = inventory_snapshot_crc32c(&root, 32, 1_000_000).unwrap();
        let count = inventory
            .iter()
            .find(|item| item.relative_path == Path::new("all_0_0_0/count.txt"))
            .unwrap();
        let expected = fs::read(part.join("count.txt")).unwrap();
        assert_eq!(count.bytes, expected.len() as u64);
        assert_eq!(count.crc32c, crate::distributed_crc32c::crc32c(&expected));
        assert!(inventory_snapshot_crc32c(&root, 1, 1_000_000).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod pinned_inventory_tests {
    use super::*;
    use crate::{Column, write_wide_part};

    #[test]
    fn checksums_follow_one_selected_immutable_generation() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-pinned-inventory-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let versions = root.join(".rusthouse-versions");
        for (generation, value) in [(1, "10"), (2, "20")] {
            let dir = versions.join(format!("g{generation}"));
            fs::create_dir_all(&dir).unwrap();
            write_wide_part(
                &dir.join("all_0_0_0"),
                &[Column { name: "id".into(), data_type: "UInt64".into() }],
                &[vec![Some(value.to_owned())]],
            ).unwrap();
        }
        fs::write(root.join(".rusthouse-active"), "g1\n").unwrap();
        let first = inventory_snapshot_crc32c(&root, 128, 1_000_000).unwrap();
        fs::write(root.join(".rusthouse-active"), "g2\n").unwrap();
        let second = inventory_snapshot_crc32c(&root, 128, 1_000_000).unwrap();
        assert_eq!(first.len(), second.len());
        assert_ne!(first, second);
        fs::remove_dir_all(root).unwrap();
    }
}
